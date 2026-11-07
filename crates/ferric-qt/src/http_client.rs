use std::sync::LazyLock;

/// Shared multi-thread Tokio runtime for all synchronous qinvokable HTTP
/// calls. Built once, first use. If this fails to build, the process cannot
/// talk to the daemon at all, so panicking here (rather than threading a
/// Result through every one of the ~50 call sites) is the correct failure
/// mode — matches this file's neighbors' existing tolerance for a
/// process-fatal panic on unrecoverable init failure.
pub(crate) static RUNTIME: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().expect("failed to build shared tokio runtime"));

/// Every `CommandBus` qinvokable answers QML by blocking the Qt UI thread on
/// its HTTP call, so a request that never returns is an indefinitely frozen
/// window — Windows then kills it as AppHangB1 (observed 2026-07-18). This
/// bounds how long a stalled daemon can freeze the UI.
///
/// ponytail: blanket client timeout, not async bus calls. The real fix is
/// making CommandBus deliver results via signals instead of `block_on`; until
/// then a stall still freezes the UI, just for a bounded interval.
///
/// This is the backstop for user-initiated *operations* — batch start/pause,
/// recheck, stage, delete. Those are legitimately slow (`batch_torrents` starts
/// N torrents sequentially, each with its own privacy check and sidecar call),
/// so cutting them off early would turn a slow success into a spurious failure.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Timeout for the reads that fire on every selection, tab switch and refresh
/// poll. These are small local queries with no reason to take seconds, and they
/// are what makes the window feel frozen when the daemon stalls — Windows ghosts
/// a window after ~5s without a pumped message, so anything longer here is
/// already a visible hang. Applied per request by the generic JSON helpers.
pub(crate) const INTERACTIVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);

/// Endpoints that do outbound network work (media extraction, magnet metadata,
/// grabber crawls) legitimately outrun `REQUEST_TIMEOUT`. They opt out per
/// request via `RequestBuilder::timeout`, which overrides the client default.
pub(crate) const LONG_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Shared HTTP client — `reqwest::Client` is Arc-backed and designed to be
/// cloned and reused, not rebuilt per call (rebuilding discards the
/// connection pool every time).
pub(crate) static HTTP_CLIENT: LazyLock<reqwest::Client> =
    LazyLock::new(|| build_client(REQUEST_TIMEOUT));

/// Separate client for the SSE progress stream. `reqwest`'s timeout covers the
/// whole request *including the response body*, so `HTTP_CLIENT` would sever a
/// live `/events` stream every `REQUEST_TIMEOUT`. This one is deliberately
/// untimed; it runs on the ProgressHub thread, never the UI thread, so it
/// carries no AppHang risk.
pub(crate) static STREAM_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

fn build_client(timeout: std::time::Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

#[cfg(test)]
mod tests {
    use super::build_client;
    use std::time::Duration;

    /// A daemon that accepts the connection and then never answers is exactly
    /// the AppHangB1 case: without a client timeout this request never returns
    /// and the UI thread blocked on it never returns either.
    #[test]
    fn client_timeout_bounds_a_black_hole_daemon() {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind black-hole listener");
        let addr = listener.local_addr().expect("listener addr");
        // Accept and hold: no response is ever written.
        std::thread::spawn(move || {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept() {
                held.push(stream);
            }
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let err = runtime
            .block_on(async move {
                build_client(Duration::from_millis(300))
                    .get(format!("http://{addr}/health"))
                    .send()
                    .await
            })
            .expect_err("black-hole daemon must not return a response");

        assert!(err.is_timeout(), "expected a timeout error, got {err:?}");
    }
}
