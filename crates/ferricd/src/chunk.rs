use crate::db;
use crate::diagnostics;
use crate::disk::DiskWriter;
use crate::security;
use crate::throttle::BandwidthLimiter;
use anyhow::{Context, Result};
use base64::Engine as _;
use bytes::Bytes;
use ferric_common::{ChunkRecord, ChunkStatus, FerricError};
use http_body_util::{BodyExt, Empty};
use hyper_util::client::legacy::connect::HttpConnector;
use reqwest::header::{
    HeaderName, HeaderValue, ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE,
    COOKIE, ETAG, LAST_MODIFIED,
};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use uuid::Uuid;

const PROGRESS_BYTES_INTERVAL: u64 = 1_048_576;

/// Bytes re-fetched before the resume point and compared with what is already
/// on disk (a reconnect rewinds a few bytes to verify); a mismatch means the saved
/// progress cannot be trusted and the piece restarts from its first byte.
const RESUME_OVERLAP: u64 = 64;

const IDLE_TIMEOUT_DEFAULT_MS: u64 = 30_000;

#[cfg(test)]
thread_local! {
    // Per-test override (tests run on their own single-threaded runtime), so a
    // test that wants a short timeout cannot make unrelated tests flaky.
    static IDLE_TIMEOUT_MS: std::cell::Cell<u64> = const { std::cell::Cell::new(IDLE_TIMEOUT_DEFAULT_MS) };
}

/// Connect/idle timeout: no response headers or no body bytes for this long
/// means the connection is dead and the piece is retried (30 s).
pub fn idle_timeout() -> Duration {
    #[cfg(test)]
    {
        Duration::from_millis(IDLE_TIMEOUT_MS.with(std::cell::Cell::get))
    }
    #[cfg(not(test))]
    {
        Duration::from_millis(IDLE_TIMEOUT_DEFAULT_MS)
    }
}

#[cfg(test)]
pub fn set_idle_timeout_ms(ms: u64) {
    IDLE_TIMEOUT_MS.with(|c| c.set(ms));
}

/// The server issued a new authentication challenge; repeat the request once.
#[derive(Debug)]
struct AuthRetry;

impl std::fmt::Display for AuthRetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("server issued a new authentication challenge")
    }
}

impl std::error::Error for AuthRetry {}

/// Saved progress disagreed with the bytes on disk; the piece restarts from 0.
#[derive(Debug)]
struct ResumeMismatch;

impl std::fmt::Display for ResumeMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("resume overlap did not match data on disk")
    }
}

impl std::error::Error for ResumeMismatch {}
const PROGRESS_TIME_INTERVAL: Duration = Duration::from_secs(1);

pub type HyperClient = hyper_util::client::legacy::Client<
    hyper_rustls::HttpsConnector<HttpConnector<crate::dns::CachingResolver>>,
    Empty<Bytes>,
>;

pub fn make_hyper_client() -> Result<HyperClient> {
    ensure_rustls_provider();
    // HTTP/2 preferred via ALPN; silent HTTP/1.1 fallback when server lacks h2. (Checked: pieces do not get
    // multiplexed onto one connection even over h2, see `pieces_get_their_own_connection_even_over_http2`.)
    // DNS: CachingResolver rotates failed A/AAAA records on the next connect.
    let mut http = HttpConnector::new_with_resolver(crate::dns::CachingResolver::global());
    http.enforce_http(false);
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()?
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .wrap_connector(http);
    Ok(
        hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build(https),
    )
}

fn ensure_rustls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Primary URL + mirrors, deduped (primary first).
pub fn build_sources(primary: &str, mirrors: &[String]) -> Vec<String> {
    let mut sources = Vec::with_capacity(1 + mirrors.len());
    sources.push(primary.to_string());
    for mirror in mirrors {
        if mirror.is_empty() || sources.iter().any(|s| s == mirror) {
            continue;
        }
        sources.push(mirror.clone());
    }
    sources
}

/// Round-robin source index for a chunk among live sources.
pub fn assign_source_index(chunk_index: u32, live_count: usize) -> usize {
    if live_count == 0 {
        return 0;
    }
    chunk_index as usize % live_count
}

/// Live mirror pool: failed sources drop out; remaining chunks redistribute.
#[derive(Debug)]
pub struct MirrorSet {
    sources: Vec<String>,
    dead: std::sync::Mutex<std::collections::HashSet<usize>>,
}

impl MirrorSet {
    pub fn new(sources: Vec<String>) -> Self {
        Self {
            sources,
            dead: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Pick a live source for `chunk_index` (round-robin over survivors).
    pub fn pick(&self, chunk_index: u32) -> Option<(usize, String)> {
        let dead = self
            .dead
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let live: Vec<usize> = (0..self.sources.len())
            .filter(|i| !dead.contains(i))
            .collect();
        if live.is_empty() {
            return None;
        }
        let slot = assign_source_index(chunk_index, live.len());
        let idx = live[slot];
        Some((idx, self.sources[idx].clone()))
    }

    pub fn mark_dead(&self, index: usize) {
        if index < self.sources.len() {
            self.dead
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(index);
        }
    }

    pub fn live_count(&self) -> usize {
        let dead = self
            .dead
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.sources.len().saturating_sub(dead.len())
    }
}

#[derive(Debug, Clone)]
pub struct ChunkRange {
    pub chunk_index: u32,
    pub byte_start: u64,
    pub byte_end: u64,
}

#[derive(Debug)]
pub enum ChunkPlan {
    Segmented {
        file_size: u64,
        chunks: Vec<ChunkRange>,
        /// Number of concurrent workers to run over `chunks` (the piece queue).
        /// Decoupled from `chunks.len()`: many small pieces, few connections.
        concurrency: usize,
        etag: Option<String>,
        last_modified: Option<String>,
        metadata: ResponseMetadata,
    },
    SingleStream {
        // Captured for parity with `Segmented`; not used for resume validation
        // on single-stream downloads (no Range requests to revalidate).
        etag: Option<String>,
        last_modified: Option<String>,
        metadata: ResponseMetadata,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ResponseMetadata {
    /// `WWW-Authenticate` answered during planning (so ranged connections can
    /// authenticate on their first request).
    pub www_authenticate: Option<String>,
    pub final_url: Option<String>,
    pub redirect_chain: Option<String>,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
}

/// Live view of a piece that is being downloaded, shared between its owner
/// (the download loop) and thieves (idle workers that take the upper half of
/// the biggest unfinished piece, IDM-style). `end` is inclusive.
#[derive(Debug)]
pub struct LiveChunk {
    pub byte_start: u64,
    end: std::sync::atomic::AtomicU64,
    done: std::sync::atomic::AtomicU64,
}

impl LiveChunk {
    pub fn new(byte_start: u64, byte_end: u64, done: u64) -> Self {
        Self {
            byte_start,
            end: std::sync::atomic::AtomicU64::new(byte_end),
            done: std::sync::atomic::AtomicU64::new(done),
        }
    }

    pub fn end(&self) -> u64 {
        self.end.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_end(&self, end: u64) {
        self.end.store(end, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn done(&self) -> u64 {
        self.done.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_done(&self, done: u64) {
        self.done.store(done, std::sync::atomic::Ordering::SeqCst);
    }

    /// Bytes this piece still has to fetch.
    pub fn remaining(&self) -> u64 {
        (self.end() + 1).saturating_sub(self.byte_start + self.done())
    }
}

/// Last byte (inclusive) this download should write for the piece right now.
fn end_now(args: &ChunkArgs) -> u64 {
    args.live.as_ref().map_or(args.chunk.byte_end, |l| l.end())
}

#[derive(Clone)]
pub struct ChunkArgs {
    pub hyper_client: HyperClient,
    pub pool: SqlitePool,
    pub disk: DiskWriter,
    pub download_id: Uuid,
    pub chunk: ChunkRecord,
    pub url: String,
    pub bandwidth: Arc<BandwidthLimiter>,
    pub cancel_rx: watch::Receiver<bool>,
    pub cookies: Option<String>,
    pub extra_headers: Option<HashMap<String, String>>,
    /// When set, all Range requests are routed through this proxy-aware reqwest
    /// client instead of the direct hyper client.
    pub proxy_client: Option<reqwest::Client>,
    /// Shared progress/end of this piece; `Some` only for HTTP(S) pieces that
    /// other workers may steal from.
    pub live: Option<Arc<LiveChunk>>,
    /// Shared credentials/challenge state for this download.
    pub auth: Option<Arc<crate::auth::AuthState>>,
    /// Live response of the planning GET (IDM's "connection 0"). Only the first
    /// attempt of piece 0 takes it; retries open a fresh ranged request.
    pub first: Option<FirstResponse>,
    /// `If-Range` value (strong ETag or Last-Modified) so a file that changed on
    /// the server is answered with a full 200 instead of mismatching bytes.
    pub validator: Option<String>,
}

/// Shared slot holding the planning response until piece 0 claims it.
pub type FirstResponse = Arc<std::sync::Mutex<Option<reqwest::Response>>>;

impl ChunkArgs {
    fn take_first_response(&self) -> Option<reqwest::Response> {
        if self.chunk.chunk_index != 0 || self.chunk.downloaded != 0 {
            return None;
        }
        self.first
            .as_ref()?
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

/// Strongest validator usable in `If-Range`: a strong ETag, else Last-Modified.
/// (Weak ETags are never valid in `If-Range`; sending one would force full 200s.)
pub fn range_validator(etag: Option<&str>, last_modified: Option<&str>) -> Option<String> {
    etag.filter(|e| !e.trim_start().starts_with("W/"))
        .or(last_modified)
        .map(str::to_string)
}

/// True when a ranged response comes from a different version of the file than the one the download
/// started with (its strong ETag, else Last-Modified, differs). A server that ignores `If-Range` sends
/// such bytes with a plain 206; keeping them would splice two versions into one corrupt file.
pub fn response_changed(expected: Option<&str>, headers: &reqwest::header::HeaderMap) -> bool {
    let Some(expected) = expected else {
        return false;
    };
    let etag = header_string(headers, ETAG);
    let last_modified = header_string(headers, LAST_MODIFIED);
    range_validator(etag.as_deref(), last_modified.as_deref()).is_some_and(|now| now != expected)
}

/// Metadata probe used when a download is added (HEAD first, ranged-GET fallback).
pub async fn plan(
    url: &str,
    client: &reqwest::Client,
    cookies: Option<&str>,
    extra_headers: Option<&HashMap<String, String>>,
    max_connections: usize,
) -> Result<ChunkPlan> {
    plan_inner(url, client, cookies, extra_headers, max_connections, false)
        .await
        .map(|(plan, _)| plan)
}

/// IDM-style planning for the job itself: the first request is a plain GET (no
/// HEAD, no `Range`) that both probes the file and, for segmented downloads,
/// is returned still open so piece 0 can use it as connection 0.
pub async fn plan_first_get(
    url: &str,
    client: &reqwest::Client,
    cookies: Option<&str>,
    extra_headers: Option<&HashMap<String, String>>,
    max_connections: usize,
) -> Result<(ChunkPlan, Option<reqwest::Response>)> {
    plan_inner(url, client, cookies, extra_headers, max_connections, true).await
}

#[allow(clippy::too_many_lines)]
async fn plan_inner(
    url: &str,
    client: &reqwest::Client,
    cookies: Option<&str>,
    extra_headers: Option<&HashMap<String, String>>,
    max_connections: usize,
    first_get: bool,
) -> Result<(ChunkPlan, Option<reqwest::Response>)> {
    let creds = credentials_from_url(url);
    let request_url = strip_userinfo(url);
    let first_method = if first_get { "GET" } else { "HEAD" };
    let send_head = || {
        let mut head = if first_get {
            // IDM sends `Accept-Encoding: identity`; a compressed body would not match the file.
            client
                .get(&request_url)
                .header("Accept-Encoding", "identity")
        } else {
            client.head(&request_url)
        };
        if let Some((user, pass)) = &creds {
            head = head.header("Authorization", basic_auth_value(user, pass));
        }
        apply_request_headers(head, cookies, extra_headers).send()
    };
    // IDM repeats a failed first request once, immediately, before giving up.
    let mut first = send_head().await;
    // (A blocked redirect is a policy decision, not a transient failure.)
    if first
        .as_ref()
        .map_or_else(|e| !e.is_redirect(), |r| r.status().is_server_error())
    {
        first = send_head().await;
    }
    let response = first?;

    // Retry once with credentials from URL — Basic, Digest, or (Windows) NTLM.
    let mut challenge: Option<String> = None;
    let response = if response.status().as_u16() == 401 {
        if let Some((user, pass)) = credentials_from_url(url) {
            let www_auth = response
                .headers()
                .get("WWW-Authenticate")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("Basic realm=\"\"")
                .to_string();
            challenge = Some(www_auth.clone());
            auth_retry(
                client,
                first_method,
                &request_url,
                cookies,
                extra_headers,
                None,
                &www_auth,
                &user,
                &pass,
            )
            .await?
        } else {
            response
        }
    } else {
        response
    };

    // Many servers/CDNs refuse HEAD (400/403/405/501) or sign URLs per method.
    // IDM never sends HEAD; probe with a one-byte ranged GET instead and keep
    // the HEAD answer only if the probe fails too.
    let mut probe_total: Option<u64> = None;
    let response = if !first_get && matches!(response.status().as_u16(), 400 | 403 | 405 | 501) {
        let mut probe_req = client.get(&request_url).header("Range", "bytes=0-0");
        if let Some((user, pass)) = &creds {
            probe_req = probe_req.header("Authorization", basic_auth_value(user, pass));
        }
        let probe = apply_request_headers(probe_req, cookies, extra_headers)
            .send()
            .await;
        match probe {
            Ok(p) if p.status().as_u16() == 206 || p.status().is_success() => {
                if p.status().as_u16() == 206 {
                    probe_total = p
                        .headers()
                        .get("content-range")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.rsplit('/').next())
                        .and_then(|v| v.trim().parse::<u64>().ok());
                }
                p
            }
            _ => response,
        }
    } else {
        response
    };

    let status = response.status().as_u16();
    if status == 403 || status == 410 {
        if security::is_signed_url_str(url) {
            return Err(FerricError::SignedUrlExpired.into());
        }
        return Err(FerricError::Forbidden.into());
    }
    if status == 404 {
        return Err(FerricError::NotFound.into());
    }
    if status == 429 {
        return Err(FerricError::RateLimited.into());
    }
    if !response.status().is_success() {
        return Err(FerricError::from_status(
            response.status().as_u16(),
            "metadata request failed",
        )
        .into());
    }

    let headers = response.headers();
    let etag = header_string(headers, ETAG);
    let last_modified = header_string(headers, LAST_MODIFIED);
    let final_url = response.url().to_string();
    let metadata = ResponseMetadata {
        www_authenticate: challenge,
        redirect_chain: redirect_chain_json(url, &final_url),
        final_url: Some(final_url),
        filename: filename_from_content_disposition(
            headers
                .get(CONTENT_DISPOSITION)
                .and_then(|value| value.to_str().ok()),
        ),
        mime_type: header_string(headers, CONTENT_TYPE)
            .map(|value| value.split(';').next().unwrap_or(&value).trim().to_string())
            .filter(|value| !value.is_empty()),
    };
    let accepts_ranges = probe_total.is_some()
        || headers
            .get(ACCEPT_RANGES)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("bytes"));
    let total = probe_total.or_else(|| {
        headers
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
    });

    // A content-encoded answer is not the file's bytes: never reuse it as piece 0 or split it.
    let encoded = headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| !v.trim().eq_ignore_ascii_case("identity"));
    // NTLM authenticates one TCP connection, not the job: no parallel pieces.
    let ntlm = metadata
        .www_authenticate
        .as_deref()
        .is_some_and(ntlm_auth::is_ntlm);
    match (accepts_ranges, total) {
        (true, Some(file_size)) if file_size > 0 && !ntlm && !encoded => {
            let concurrency = planned_chunk_count(file_size, max_connections);
            let chunks = plan_pieces(file_size, concurrency);
            // Keep the planning GET open only when it really streams the file from byte 0.
            let first = (first_get && status == 200 && chunks.len() > 1).then_some(response);
            Ok((
                ChunkPlan::Segmented {
                    file_size,
                    chunks,
                    concurrency,
                    etag,
                    last_modified,
                    metadata,
                },
                first,
            ))
        }
        _ => Ok((
            ChunkPlan::SingleStream {
                etag,
                last_modified,
                metadata,
            },
            // The stream from byte 0 is the whole file: reuse it so one-time URLs survive.
            (first_get && status == 200 && !ntlm).then_some(response),
        )),
    }
}

fn note_dns_failure(url: &str) {
    if let Some(host) = crate::dns::host_from_url(url) {
        // Bust cache so the next connect re-resolves and can pick another A/AAAA.
        crate::dns::CachingResolver::global().invalidate(&host);
    }
}

/// Parse RFC 9530-ish `Content-Digest: sha-256=:base64:` (also plain hex after algo).
/// Returns (algorithm, `expected_hex_lowercase`).
pub(crate) fn parse_content_digest(header: &str) -> Option<(DigestAlg, String)> {
    // Take first algorithm=value pair.
    let part = header.split(',').next()?.trim();
    let (alg_raw, rest) = part.split_once('=')?;
    let alg = DigestAlg::parse(alg_raw.trim())?;
    let rest = rest.trim();
    let digest = if rest.starts_with(':') && rest.ends_with(':') && rest.len() >= 2 {
        let b64 = &rest[1..rest.len() - 1];
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
        hex_encode(&bytes)
    } else {
        rest.trim_matches(':').to_ascii_lowercase()
    };
    if digest.is_empty() {
        return None;
    }
    Some((alg, digest))
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum DigestAlg {
    Sha256,
    Sha1,
    Md5,
}

impl DigestAlg {
    fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().replace('_', "-").as_str() {
            "sha-256" | "sha256" => Some(Self::Sha256),
            "sha-1" | "sha1" => Some(Self::Sha1),
            "md5" => Some(Self::Md5),
            _ => None,
        }
    }
}

enum BodyHasher {
    Sha256(sha2::Sha256),
    Sha1(sha1::Sha1),
    Md5(md5::Context),
}

impl BodyHasher {
    fn new(alg: &DigestAlg) -> Self {
        use sha1::{Digest as _, Sha1};
        use sha2::Sha256;
        match alg {
            DigestAlg::Sha256 => Self::Sha256(Sha256::new()),
            DigestAlg::Sha1 => Self::Sha1(Sha1::new()),
            DigestAlg::Md5 => Self::Md5(md5::Context::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        use sha1::Digest as _;
        match self {
            Self::Sha256(h) => h.update(data),
            Self::Sha1(h) => h.update(data),
            Self::Md5(h) => h.consume(data),
        }
    }

    fn finalize_hex(self) -> String {
        use sha1::Digest as _;
        match self {
            Self::Sha256(h) => format!("{:x}", h.finalize()),
            Self::Sha1(h) => format!("{:x}", h.finalize()),
            Self::Md5(h) => format!("{:x}", h.compute()),
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[allow(clippy::too_many_lines)]
pub async fn download_chunk(args: ChunkArgs) -> Result<u64> {
    let first = match download_chunk_once(args.clone()).await {
        Err(e) if e.downcast_ref::<AuthRetry>().is_some() => {
            download_chunk_once(args.clone()).await
        }
        other => other,
    };
    match first {
        Err(e) if e.downcast_ref::<ResumeMismatch>().is_some() => {
            tracing::warn!(
                chunk = args.chunk.chunk_index,
                "resume overlap mismatch; restarting piece"
            );
            let mut fresh = args;
            fresh.chunk.downloaded = 0;
            download_chunk_once(fresh).await
        }
        other => other,
    }
}

async fn download_chunk_once(args: ChunkArgs) -> Result<u64> {
    if let Some(ref client) = args.proxy_client {
        return download_chunk_reqwest(client.clone(), &args).await;
    }
    if *args.cancel_rx.borrow() {
        return Err(FerricError::Cancelled.into());
    }

    db::chunks::update_chunk_progress(
        &args.pool,
        args.chunk.id,
        args.chunk.downloaded,
        ChunkStatus::Downloading,
    )
    .await?;

    if let Some(first) = args.take_first_response() {
        return consume_reqwest_stream(first, &args, 0).await;
    }

    let verify_len = if args.chunk.downloaded >= RESUME_OVERLAP {
        RESUME_OVERLAP
    } else {
        0
    };
    let start = args.chunk.resume_offset() - verify_len;
    let range = format!("bytes={start}-{}", end_now(&args));
    let mut overlap: Vec<u8> = Vec::new();
    let mut truncated = false;

    let mut req_builder = hyper::Request::builder()
        .method("GET")
        .uri(strip_userinfo(&args.url))
        .header("Range", range);
    if let Some(validator) = &args.validator {
        req_builder = req_builder.header("If-Range", validator.as_str());
    }

    if let Some(cookie) = &args.cookies {
        req_builder = req_builder.header("Cookie", cookie.as_str());
    }
    if let Some(auth) = &args.auth {
        req_builder = req_builder.header("Authorization", auth.header("GET", &args.url));
    }
    if let Some(extra) = &args.extra_headers {
        for (name, value) in extra {
            if let (Ok(name), Ok(val)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                req_builder = req_builder.header(name, val);
            }
        }
    }

    let req = req_builder.body(Empty::<Bytes>::new())?;
    let request_started = Instant::now();
    diagnostics::event(
        "chunk_request_start",
        serde_json::json!({
            "download_id": args.download_id,
            "chunk": args.chunk.chunk_index,
            "byte_start": args.chunk.byte_start,
            "byte_end": args.chunk.byte_end,
            "already_downloaded": args.chunk.downloaded,
        }),
    )
    .await;
    let response = match tokio::time::timeout(idle_timeout(), args.hyper_client.request(req)).await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            note_dns_failure(&args.url);
            return Err(e.into());
        }
        Err(_) => {
            return Err(anyhow::anyhow!(
                "error reading a body from connection: connect timed out"
            ));
        }
    };
    let status = response.status().as_u16();

    if status == 401 {
        if let (Some(auth), Some(www)) = (
            &args.auth,
            response
                .headers()
                .get("www-authenticate")
                .and_then(|v| v.to_str().ok()),
        ) {
            if auth.learn_challenge(www) {
                return Err(AuthRetry.into());
            }
        }
    }

    if status == 429 {
        db::chunks::update_chunk_progress(
            &args.pool,
            args.chunk.id,
            args.chunk.downloaded,
            ChunkStatus::Error,
        )
        .await?;
        return Err(FerricError::RateLimited.into());
    }

    if status >= 500 {
        // 502/503/504 mid-download usually means "too many connections right now", not "no Range support":
        // retry the piece with back-off instead of giving up parallel download for the whole file.
        db::chunks::update_chunk_progress(
            &args.pool,
            args.chunk.id,
            args.chunk.downloaded,
            ChunkStatus::Error,
        )
        .await?;
        return Err(FerricError::from_status(status, "server busy, retrying").into());
    }
    if status != 206 {
        db::chunks::update_chunk_progress(
            &args.pool,
            args.chunk.id,
            args.chunk.downloaded,
            ChunkStatus::Error,
        )
        .await?;
        return Err(FerricError::RangeRejected.into());
    }

    if response_changed(args.validator.as_deref(), response.headers()) {
        db::chunks::update_chunk_progress(
            &args.pool,
            args.chunk.id,
            args.chunk.downloaded,
            ChunkStatus::Error,
        )
        .await?;
        // Like IDM ("started to download the file using one connection because of data corruption"):
        // the file changed under us, so start over as one consistent stream.
        return Err(FerricError::RangeRejected.into());
    }

    let content_digest = response
        .headers()
        .get("content-digest")
        .or_else(|| response.headers().get("Content-Digest"))
        .and_then(|v| v.to_str().ok())
        .and_then(parse_content_digest);

    let mut body = response.into_body();
    let mut written = args.chunk.downloaded;
    let mut unflushed = 0_u64;
    let mut last_flush = Instant::now();
    let mut saw_first_byte = false;
    let mut body_hasher = content_digest.as_ref().map(|(alg, _)| BodyHasher::new(alg));

    loop {
        if *args.cancel_rx.borrow() {
            db::chunks::update_chunk_progress(
                &args.pool,
                args.chunk.id,
                written,
                ChunkStatus::Pending,
            )
            .await?;
            return Err(FerricError::Cancelled.into());
        }

        let expected_now = end_now(&args) - args.chunk.byte_start + 1;
        if written >= expected_now {
            truncated = args.live.is_some();
            break;
        }
        let frame = tokio::time::timeout(idle_timeout(), body.frame())
            .await
            .map_err(|_| anyhow::anyhow!("error reading a body from connection: idle timeout"))?;
        let Some(frame) = frame.transpose().map_err(|error| {
            note_dns_failure(&args.url);
            anyhow::anyhow!("error reading a body from connection: {error}")
        })?
        else {
            break;
        };

        let Ok(mut data) = frame.into_data() else {
            continue; // skip trailer frames
        };
        if verify_overlap(&args, verify_len, &mut overlap, &mut data).await? {
            continue;
        }
        let allowed = expected_now.saturating_sub(written);
        if (data.len() as u64) > allowed {
            data = data.slice(..allowed as usize);
            truncated = true;
        }
        if !saw_first_byte {
            saw_first_byte = true;
            diagnostics::event(
                "chunk_first_byte",
                serde_json::json!({
                    "download_id": args.download_id,
                    "chunk": args.chunk.chunk_index,
                    "latency_ms": request_started.elapsed().as_millis(),
                    "first_frame_bytes": data.len(),
                }),
            )
            .await;
        }
        if let Some(hasher) = body_hasher.as_mut() {
            hasher.update(&data);
        }
        args.bandwidth.acquire(data.len()).await;
        // The wait for tokens can be long under a speed limit; another connection may have taken over the
        // tail of this piece meanwhile (work stealing). Do not write bytes that are no longer ours.
        let allowed = (end_now(&args) - args.chunk.byte_start + 1).saturating_sub(written);
        if (data.len() as u64) > allowed {
            data = data.slice(..allowed as usize);
        }
        if data.is_empty() {
            continue;
        }
        args.disk
            .write_at(args.chunk.byte_start + written, &data)
            .await?;
        let len = u64::try_from(data.len()).context("chunk length exceeds u64")?;
        written = written.saturating_add(len);
        if let Some(live) = &args.live {
            live.set_done(written);
        }
        unflushed = unflushed.saturating_add(len);

        // Pieces of a segmented download (`live` is set) do not stop to flush: the job's checkpointer
        // flushes the file on its own clock and then saves their progress (see `job::checkpoint`).
        if args.live.is_none()
            && (unflushed >= PROGRESS_BYTES_INTERVAL
                || last_flush.elapsed() >= PROGRESS_TIME_INTERVAL)
        {
            // Data first, bookkeeping second: a saved offset must never be ahead of the file.
            args.disk.sync().await?;
            db::chunks::update_chunk_progress(
                &args.pool,
                args.chunk.id,
                written,
                ChunkStatus::Downloading,
            )
            .await?;
            db::downloads::update_progress(
                &args.pool,
                args.download_id,
                total_downloaded(&args.pool, args.download_id).await?,
            )
            .await?;
            unflushed = 0;
            last_flush = Instant::now();
        }
    }

    let expected_len = end_now(&args) - args.chunk.byte_start + 1;
    let final_status = if written >= expected_len {
        ChunkStatus::Completed
    } else {
        ChunkStatus::Error
    };
    args.disk.sync().await?;
    db::chunks::update_chunk_progress(&args.pool, args.chunk.id, written, final_status).await?;

    if final_status == ChunkStatus::Error {
        diagnostics::event(
            "chunk_response_incomplete",
            serde_json::json!({
                "download_id": args.download_id,
                "chunk": args.chunk.chunk_index,
                "written": written,
                "expected": expected_len,
                "saw_first_byte": saw_first_byte,
            }),
        )
        .await;
        return Err(FerricError::ChunkFailed {
            index: args.chunk.chunk_index as usize,
            reason: "response ended before range was complete".to_string(),
        }
        .into());
    }

    if let (Some((_, expected)), Some(hasher)) =
        (content_digest.as_ref(), body_hasher.filter(|_| !truncated))
    {
        let actual = hasher.finalize_hex();
        if !actual.eq_ignore_ascii_case(expected) {
            db::chunks::update_chunk_progress(&args.pool, args.chunk.id, 0, ChunkStatus::Error)
                .await?;
            return Err(FerricError::ChunkFailed {
                index: args.chunk.chunk_index as usize,
                reason: "content-digest mismatch".to_string(),
            }
            .into());
        }
    }

    if let Some(host) = crate::dns::host_from_url(&args.url) {
        crate::dns::CachingResolver::global().mark_ok(&host);
    }

    Ok(written)
}

/// reqwest-based parallel to [`download_chunk`] used when a proxy client is set.
#[allow(clippy::too_many_lines)]
async fn download_chunk_reqwest(client: reqwest::Client, args: &ChunkArgs) -> Result<u64> {
    if *args.cancel_rx.borrow() {
        return Err(FerricError::Cancelled.into());
    }
    db::chunks::update_chunk_progress(
        &args.pool,
        args.chunk.id,
        args.chunk.downloaded,
        ChunkStatus::Downloading,
    )
    .await?;
    if let Some(first) = args.take_first_response() {
        return consume_reqwest_stream(first, args, 0).await;
    }

    let verify_len = if args.chunk.downloaded >= RESUME_OVERLAP {
        RESUME_OVERLAP
    } else {
        0
    };
    let start = args.chunk.resume_offset() - verify_len;
    let range = format!("bytes={start}-{}", end_now(args));
    let mut req = client.get(strip_userinfo(&args.url)).header("Range", range);
    if let Some(validator) = &args.validator {
        req = req.header("If-Range", validator.as_str());
    }
    if let Some(cookie) = &args.cookies {
        req = req.header(COOKIE, cookie.as_str());
    }
    if let Some(auth) = &args.auth {
        req = req.header("Authorization", auth.header("GET", &args.url));
    }
    if let Some(extra) = &args.extra_headers {
        for (name, value) in extra {
            if let (Ok(name), Ok(val)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                req = req.header(name, val);
            }
        }
    }
    let response = tokio::time::timeout(idle_timeout(), req.send())
        .await
        .map_err(|_| {
            anyhow::anyhow!("error reading a body from connection: connect timed out")
        })??;
    let status = response.status().as_u16();
    if status == 401 {
        if let (Some(auth), Some(www)) = (
            &args.auth,
            response
                .headers()
                .get("www-authenticate")
                .and_then(|v| v.to_str().ok()),
        ) {
            if auth.learn_challenge(www) {
                return Err(AuthRetry.into());
            }
        }
    }
    if status == 429 {
        db::chunks::update_chunk_progress(
            &args.pool,
            args.chunk.id,
            args.chunk.downloaded,
            ChunkStatus::Error,
        )
        .await?;
        return Err(FerricError::RateLimited.into());
    }
    if status >= 500 {
        // 502/503/504 mid-download usually means "too many connections right now", not "no Range support":
        // retry the piece with back-off instead of giving up parallel download for the whole file.
        db::chunks::update_chunk_progress(
            &args.pool,
            args.chunk.id,
            args.chunk.downloaded,
            ChunkStatus::Error,
        )
        .await?;
        return Err(FerricError::from_status(status, "server busy, retrying").into());
    }
    if status != 206 {
        db::chunks::update_chunk_progress(
            &args.pool,
            args.chunk.id,
            args.chunk.downloaded,
            ChunkStatus::Error,
        )
        .await?;
        return Err(FerricError::RangeRejected.into());
    }

    if response_changed(args.validator.as_deref(), response.headers()) {
        db::chunks::update_chunk_progress(
            &args.pool,
            args.chunk.id,
            args.chunk.downloaded,
            ChunkStatus::Error,
        )
        .await?;
        // Like IDM ("started to download the file using one connection because of data corruption"):
        // the file changed under us, so start over as one consistent stream.
        return Err(FerricError::RangeRejected.into());
    }

    consume_reqwest_stream(response, args, verify_len).await
}

/// Write one response body into the piece, stopping at the piece's current end
/// (the response is dropped there, which closes the connection like IDM does).
async fn consume_reqwest_stream(
    response: reqwest::Response,
    args: &ChunkArgs,
    verify_len: u64,
) -> Result<u64> {
    use futures::StreamExt as _;
    let mut overlap: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    let mut written = args.chunk.downloaded;
    let mut unflushed = 0_u64;
    let mut last_flush = Instant::now();

    loop {
        let expected_now = end_now(args) - args.chunk.byte_start + 1;
        if written >= expected_now {
            break;
        }
        let next = tokio::time::timeout(idle_timeout(), stream.next())
            .await
            .map_err(|_| anyhow::anyhow!("error reading a body from connection: idle timeout"))?;
        let Some(result) = next else { break };
        if *args.cancel_rx.borrow() {
            db::chunks::update_chunk_progress(
                &args.pool,
                args.chunk.id,
                written,
                ChunkStatus::Pending,
            )
            .await?;
            return Err(FerricError::Cancelled.into());
        }
        let mut data = result.map_err(anyhow::Error::from)?;
        if verify_overlap(args, verify_len, &mut overlap, &mut data).await? {
            continue;
        }
        let allowed = expected_now.saturating_sub(written);
        if (data.len() as u64) > allowed {
            data = data.slice(..allowed as usize);
        }
        args.bandwidth.acquire(data.len()).await;
        // The wait for tokens can be long under a speed limit; another connection may have taken over the
        // tail of this piece meanwhile (work stealing). Do not write bytes that are no longer ours.
        let allowed = (end_now(args) - args.chunk.byte_start + 1).saturating_sub(written);
        if (data.len() as u64) > allowed {
            data = data.slice(..allowed as usize);
        }
        if data.is_empty() {
            continue;
        }
        args.disk
            .write_at(args.chunk.byte_start + written, &data)
            .await?;
        let len = u64::try_from(data.len()).context("chunk length exceeds u64")?;
        written = written.saturating_add(len);
        if let Some(live) = &args.live {
            live.set_done(written);
        }
        unflushed = unflushed.saturating_add(len);
        // Pieces of a segmented download (`live` is set) do not stop to flush: the job's checkpointer
        // flushes the file on its own clock and then saves their progress (see `job::checkpoint`).
        if args.live.is_none()
            && (unflushed >= PROGRESS_BYTES_INTERVAL
                || last_flush.elapsed() >= PROGRESS_TIME_INTERVAL)
        {
            // Data first, bookkeeping second: a saved offset must never be ahead of the file.
            args.disk.sync().await?;
            db::chunks::update_chunk_progress(
                &args.pool,
                args.chunk.id,
                written,
                ChunkStatus::Downloading,
            )
            .await?;
            db::downloads::update_progress(
                &args.pool,
                args.download_id,
                total_downloaded(&args.pool, args.download_id).await?,
            )
            .await?;
            unflushed = 0;
            last_flush = Instant::now();
        }
    }

    let final_status = if written > end_now(args) - args.chunk.byte_start {
        ChunkStatus::Completed
    } else {
        ChunkStatus::Error
    };
    args.disk.sync().await?;
    db::chunks::update_chunk_progress(&args.pool, args.chunk.id, written, final_status).await?;
    if final_status == ChunkStatus::Error {
        return Err(FerricError::ChunkFailed {
            index: args.chunk.chunk_index as usize,
            reason: "response ended before range was complete".to_string(),
        }
        .into());
    }
    Ok(written)
}

/// Consume the resume-overlap prefix of a response body. Returns `true` when
/// the whole `data` buffer was swallowed by the overlap (caller should skip it);
/// otherwise `data` is trimmed to the bytes that must be written. On a
/// mismatch the stored progress is reset and [`ResumeMismatch`] is returned.
async fn verify_overlap(
    args: &ChunkArgs,
    verify_len: u64,
    overlap: &mut Vec<u8>,
    data: &mut Bytes,
) -> Result<bool> {
    let want = verify_len as usize;
    if overlap.len() >= want {
        return Ok(false);
    }
    let take = (want - overlap.len()).min(data.len());
    overlap.extend_from_slice(&data[..take]);
    *data = data.slice(take..);
    if overlap.len() == want {
        let at = args.chunk.byte_start + args.chunk.downloaded - verify_len;
        let on_disk = args.disk.read_at(at, want).await?;
        if on_disk != *overlap {
            db::chunks::update_chunk_progress(&args.pool, args.chunk.id, 0, ChunkStatus::Pending)
                .await?;
            return Err(ResumeMismatch.into());
        }
    }
    Ok(data.is_empty())
}

/// The URL without `user:pass@` (credentials travel in the `Authorization`
/// header, never in the request target).
pub(crate) fn strip_userinfo(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) if !u.username().is_empty() || u.password().is_some() => {
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.to_string()
        }
        _ => url.to_string(),
    }
}

/// Extract `(username, password)` from URL userinfo (`http://user:pass@host/`).
pub(crate) fn credentials_from_url(url: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(url).ok()?;
    let raw_user = parsed.username();
    if raw_user.is_empty() {
        return None;
    }
    let user = percent_decode(raw_user).unwrap_or_else(|| raw_user.to_string());
    let raw_pass = parsed.password().unwrap_or("");
    let pass = percent_decode(raw_pass).unwrap_or_default();
    Some((user, pass))
}

/// Build an `Authorization` header value by inspecting the `WWW-Authenticate`
/// challenge and using Basic or Digest as appropriate.
pub(crate) fn build_auth_header(
    www_auth: &str,
    method: &str,
    url: &str,
    user: &str,
    pass: &str,
) -> String {
    let scheme = www_auth
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match scheme.as_str() {
        "digest" => digest_auth_value(www_auth, method, url, user, pass),
        _ => basic_auth_value(user, pass),
    }
}

pub(crate) fn basic_auth_value(user: &str, pass: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
    format!("Basic {encoded}")
}

fn digest_auth_value(www_auth: &str, method: &str, url: &str, user: &str, pass: &str) -> String {
    let cnonce = format!("{:08x}", rand::random::<u32>());
    digest_header(www_auth, method, url, user, pass, 1, &cnonce)
}

/// RFC 2617 Digest header. Uses `qop=auth` (with `nc`/`cnonce`) when the
/// challenge offers it, otherwise the RFC 2069 form; `uri` is path + query.
pub(crate) fn digest_header(
    www_auth: &str,
    method: &str,
    url: &str,
    user: &str,
    pass: &str,
    nc: u32,
    cnonce: &str,
) -> String {
    let params = parse_digest_params(www_auth);
    let realm = params.get("realm").copied().unwrap_or("");
    let nonce = params.get("nonce").copied().unwrap_or("");
    let opaque = params.get("opaque").copied();
    let use_qop = params
        .get("qop")
        .is_some_and(|q| q.split(',').any(|v| v.trim().eq_ignore_ascii_case("auth")));
    let uri = url::Url::parse(url).map_or_else(
        |_| "/".to_string(),
        |u| match u.query() {
            Some(q) => format!("{}?{q}", u.path()),
            None => u.path().to_string(),
        },
    );
    let ha1 = format!("{:x}", md5::compute(format!("{user}:{realm}:{pass}")));
    let ha2 = format!("{:x}", md5::compute(format!("{method}:{uri}")));
    let nc_hex = format!("{nc:08x}");
    let response_hex = if use_qop {
        format!(
            "{:x}",
            md5::compute(format!("{ha1}:{nonce}:{nc_hex}:{cnonce}:auth:{ha2}"))
        )
    } else {
        format!("{:x}", md5::compute(format!("{ha1}:{nonce}:{ha2}")))
    };
    let mut header =
        format!(r#"Digest username="{user}", realm="{realm}", nonce="{nonce}", uri="{uri}""#);
    if use_qop {
        let _ = write!(header, r#", qop=auth, nc={nc_hex}, cnonce="{cnonce}""#);
    }
    let _ = write!(header, r#", response="{response_hex}""#);
    if let Some(op) = opaque {
        let _ = write!(header, r#", opaque="{op}""#);
    }
    header
}

fn parse_digest_params(www_auth: &str) -> HashMap<&str, &str> {
    let mut params = HashMap::new();
    let body = www_auth
        .strip_prefix("Digest ")
        .or_else(|| www_auth.strip_prefix("digest "))
        .unwrap_or(www_auth);
    for part in body.split(',') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            params.insert(key, value);
        }
    }
    params
}

pub async fn total_downloaded(pool: &SqlitePool, download_id: Uuid) -> Result<u64> {
    let total: Option<i64> =
        sqlx::query_scalar("SELECT SUM(downloaded) FROM chunks WHERE download_id = ?1")
            .bind(download_id.to_string())
            .fetch_one(pool)
            .await?;
    Ok(total.unwrap_or(0).max(0) as u64)
}

/// The user agent for requests the browser did not describe (a download added by hand, a pasted link).
/// One stable value for every request of a download, like IDM: servers that look at it, or at changes
/// in it between ranged requests, see a normal browser.
pub(crate) fn default_user_agent() -> &'static str {
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"
}

fn apply_request_headers(
    mut builder: reqwest::RequestBuilder,
    cookies: Option<&str>,
    extra_headers: Option<&HashMap<String, String>>,
) -> reqwest::RequestBuilder {
    if let Some(cookie) = cookies {
        builder = builder.header(COOKIE, cookie);
    }
    let has_ua =
        extra_headers.is_some_and(|h| h.keys().any(|k| k.eq_ignore_ascii_case("user-agent")));
    if !has_ua {
        builder = builder.header(reqwest::header::USER_AGENT, default_user_agent());
    }
    if let Some(headers) = extra_headers {
        for (name, value) in headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                builder = builder.header(name, value);
            }
        }
    }
    builder
}

fn header_string(
    headers: &reqwest::header::HeaderMap,
    name: reqwest::header::HeaderName,
) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

/// Smallest work-stealing piece — below this the per-request overhead dominates.
pub fn redirect_chain_json(original: &str, final_url: &str) -> Option<String> {
    let original_url = url::Url::parse(original).ok()?;
    let final_url = url::Url::parse(final_url).ok()?;
    if original_url == final_url {
        return None;
    }

    serde_json::to_string(&[original_url.as_str(), final_url.as_str()]).ok()
}

/// Splitting grain: 2 x 116,800 bytes (the first extra connection starts
/// at 233,600). A piece is split only while at least this much is left, and a
/// file is never cut into initial pieces smaller than this.
pub const PIECE_GRAIN: u64 = 233_600;

/// IDM starts with 4 connections at once and keeps splitting afterwards.
const INITIAL_PIECES: u64 = 4;

/// Initial layout of a segmented download: up to 4 equal pieces. Everything
/// after that is dynamic: whenever a worker is idle it halves the in-flight
/// piece with the most bytes left (see `job::try_steal`), following a
/// "split the largest chunk" rule, so there is no fixed piece queue to drain.
pub fn plan_pieces(file_size: u64, concurrency: usize) -> Vec<ChunkRange> {
    if file_size == 0 {
        return Vec::new();
    }
    let by_size = (file_size / PIECE_GRAIN).max(1);
    let count = INITIAL_PIECES
        .min(concurrency.max(1) as u64)
        .min(by_size)
        .max(1);
    if count < INITIAL_PIECES {
        return split_into_chunks(file_size, count as usize);
    }
    // Initial layout: connection 1 starts at 0 and is cut at 233,600
    // (2 x 116,800), where connection 2 begins; connections 3 and 4 start at 1/2
    // and 3/4 of the rest (20 MB file: 233,600 / 10,6xx,xxx / 15,79x,xxx).
    let s1 = PIECE_GRAIN;
    let s2 = s1 + (file_size - s1) / 2;
    let s3 = s2 + (file_size - s2) / 2;
    [(0, s1), (s1, s2), (s2, s3), (s3, file_size)]
        .iter()
        .enumerate()
        .map(|(chunk_index, &(start, end))| ChunkRange {
            chunk_index: chunk_index as u32,
            byte_start: start,
            byte_end: end - 1,
        })
        .collect()
}

pub fn optimal_chunk_count(size: u64) -> usize {
    match size {
        0..=1_048_576 => 1,
        1_048_577..=104_857_600 => 8,
        _ => 16,
    }
}

pub fn planned_chunk_count(size: u64, max_connections: usize) -> usize {
    optimal_chunk_count(size).min(max_connections.max(1))
}

/// A server-supplied name made safe to use as a file name: only its last path component (so
/// `..\..\evil.txt` becomes `evil.txt`), characters Windows forbids replaced by `-` (so `a:b*c?d.txt`
/// keeps its extension instead of being cut at the `?`).
pub(crate) fn clean_server_filename(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = last
        .chars()
        // IDM maps '?' to '.' and the other forbidden characters to '-' (a:b*c?d.txt -> a-b-c.d.txt)
        .map(|c| match c {
            '?' => '.',
            '<' | '>' | ':' | '"' | '|' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().to_string();
    (!cleaned.is_empty() && cleaned != "." && cleaned != "..").then_some(cleaned)
}

pub fn filename_from_content_disposition(value: Option<&str>) -> Option<String> {
    filename_from_content_disposition_raw(value).and_then(|n| clean_server_filename(&n))
}

fn filename_from_content_disposition_raw(value: Option<&str>) -> Option<String> {
    let value = value?;
    let mut fallback = None;
    for part in value.split(';').skip(1) {
        let part = part.trim();
        let Some((name, raw)) = part.split_once('=') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name == "filename*" {
            if let Some(decoded) = decode_rfc5987(raw.trim()) {
                return Some(security::strip_rtl_overrides(&decoded));
            }
        }
        if name == "filename" {
            let filename = raw.trim().trim_matches('"').trim();
            if !filename.is_empty() {
                fallback = Some(security::strip_rtl_overrides(filename));
            }
        }
    }
    fallback
}

fn decode_rfc5987(value: &str) -> Option<String> {
    let value = value.trim_matches('"');
    let encoded = value
        .rsplit_once("''")
        .map_or(value, |(_, encoded)| encoded);
    percent_decode(encoded)
}

pub(crate) fn percent_decode(value: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut i = 0;
    let raw = value.as_bytes();
    while i < raw.len() {
        if raw[i] == b'%' {
            if i + 2 >= raw.len() {
                return None;
            }
            let hex = std::str::from_utf8(&raw[i + 1..i + 3]).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    String::from_utf8(bytes)
        .ok()
        .filter(|value| !value.is_empty())
}

pub fn split_into_chunks(file_size: u64, count: usize) -> Vec<ChunkRange> {
    if file_size == 0 || count == 0 {
        return Vec::new();
    }

    let chunk_size = file_size.div_ceil(count as u64);
    (0..count as u64)
        .map(|i| {
            let start = i * chunk_size;
            let end = ((i + 1) * chunk_size - 1).min(file_size - 1);
            ChunkRange {
                chunk_index: i as u32,
                byte_start: start,
                byte_end: end,
            }
        })
        .collect()
}

/// Auth retry helper: Basic/Digest for all platforms, NTLM on Windows.
/// `range` is included on every request (needed for chunk re-auth).
pub(crate) async fn auth_retry(
    client: &reqwest::Client,
    method: &str,
    url: &str,
    cookies: Option<&str>,
    extra_headers: Option<&HashMap<String, String>>,
    range: Option<&str>,
    www_auth: &str,
    user: &str,
    pass: &str,
) -> Result<reqwest::Response> {
    use reqwest::Method;
    use std::str::FromStr;

    if !method.eq_ignore_ascii_case("POST") && ntlm_auth::is_ntlm(www_auth) {
        return ntlm_auth::handshake(
            client,
            method,
            url,
            cookies,
            extra_headers,
            range,
            user,
            pass,
        )
        .await;
    }

    let auth = build_auth_header(www_auth, method, url, user, pass);
    let m = Method::from_str(method).unwrap_or(Method::GET);
    let req = client.request(m, url);
    let req = match range {
        Some(r) => req.header("Range", r),
        None => req,
    };
    apply_request_headers(req, cookies, extra_headers)
        .header("Authorization", auth)
        .send()
        .await
        .map_err(anyhow::Error::from)
}

/// `NTLMv2` (MS-NLMP) over HTTP: Type 1 -> 401 + Type 2 -> Type 3 on the same
/// keep-alive connection, which is why NTLM downloads run as a single stream
/// (the authentication belongs to one TCP connection, not to the whole job).
pub(crate) mod ntlm_auth {
    use anyhow::{anyhow, Result};
    use base64::Engine as _;
    use std::collections::HashMap;
    use std::str::FromStr;

    pub(crate) fn is_ntlm(www_auth: &str) -> bool {
        www_auth
            .split_whitespace()
            .next()
            .unwrap_or("")
            .eq_ignore_ascii_case("ntlm")
    }

    /// MD4 (RFC 1320); only used to derive the NT hash.
    pub(super) fn md4(input: &[u8]) -> [u8; 16] {
        let mut msg = input.to_vec();
        let bits = (input.len() as u64) * 8;
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bits.to_le_bytes());
        let f = |x: u32, y: u32, z: u32| (x & y) | (!x & z);
        let g = |x: u32, y: u32, z: u32| (x & y) | (x & z) | (y & z);
        let h = |x: u32, y: u32, z: u32| x ^ y ^ z;
        let (mut a, mut b, mut c, mut d) = (
            0x6745_2301u32,
            0xefcd_ab89u32,
            0x98ba_dcfeu32,
            0x1032_5476u32,
        );
        for block in msg.as_chunks::<64>().0 {
            let x: Vec<u32> = block
                .as_chunks::<4>()
                .0
                .iter()
                .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
                .collect();
            let (aa, bb, cc, dd) = (a, b, c, d);
            for i in [0, 4, 8, 12] {
                a = a.wrapping_add(f(b, c, d)).wrapping_add(x[i]).rotate_left(3);
                d = d
                    .wrapping_add(f(a, b, c))
                    .wrapping_add(x[i + 1])
                    .rotate_left(7);
                c = c
                    .wrapping_add(f(d, a, b))
                    .wrapping_add(x[i + 2])
                    .rotate_left(11);
                b = b
                    .wrapping_add(f(c, d, a))
                    .wrapping_add(x[i + 3])
                    .rotate_left(19);
            }
            for k in 0..4 {
                a = a
                    .wrapping_add(g(b, c, d))
                    .wrapping_add(x[k])
                    .wrapping_add(0x5a82_7999)
                    .rotate_left(3);
                d = d
                    .wrapping_add(g(a, b, c))
                    .wrapping_add(x[k + 4])
                    .wrapping_add(0x5a82_7999)
                    .rotate_left(5);
                c = c
                    .wrapping_add(g(d, a, b))
                    .wrapping_add(x[k + 8])
                    .wrapping_add(0x5a82_7999)
                    .rotate_left(9);
                b = b
                    .wrapping_add(g(c, d, a))
                    .wrapping_add(x[k + 12])
                    .wrapping_add(0x5a82_7999)
                    .rotate_left(13);
            }
            for k in [0, 2, 1, 3] {
                a = a
                    .wrapping_add(h(b, c, d))
                    .wrapping_add(x[k])
                    .wrapping_add(0x6ed9_eba1)
                    .rotate_left(3);
                d = d
                    .wrapping_add(h(a, b, c))
                    .wrapping_add(x[k + 8])
                    .wrapping_add(0x6ed9_eba1)
                    .rotate_left(9);
                c = c
                    .wrapping_add(h(d, a, b))
                    .wrapping_add(x[k + 4])
                    .wrapping_add(0x6ed9_eba1)
                    .rotate_left(11);
                b = b
                    .wrapping_add(h(c, d, a))
                    .wrapping_add(x[k + 12])
                    .wrapping_add(0x6ed9_eba1)
                    .rotate_left(15);
            }
            a = a.wrapping_add(aa);
            b = b.wrapping_add(bb);
            c = c.wrapping_add(cc);
            d = d.wrapping_add(dd);
        }
        let mut out = [0u8; 16];
        for (i, v) in [a, b, c, d].iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }

    pub(super) fn hmac_md5(key: &[u8], data: &[u8]) -> [u8; 16] {
        let mut k = [0u8; 64];
        if key.len() > 64 {
            k[..16].copy_from_slice(&md5::compute(key).0);
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let mut inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
        inner.extend_from_slice(data);
        let mut outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
        outer.extend_from_slice(&md5::compute(&inner).0);
        md5::compute(&outer).0
    }

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    /// `NTOWFv2` = `HMAC_MD5(MD4(UTF16(password))`, UTF16(upper(user) + domain)).
    pub(super) fn ntowf_v2(user: &str, domain: &str, pass: &str) -> [u8; 16] {
        let nt = md4(&utf16le(pass));
        hmac_md5(&nt, &utf16le(&format!("{}{}", user.to_uppercase(), domain)))
    }

    pub(super) fn type1_message() -> Vec<u8> {
        let mut m = b"NTLMSSP\0".to_vec();
        m.extend_from_slice(&1u32.to_le_bytes());
        // UNICODE | OEM | REQUEST_TARGET | NTLM | ALWAYS_SIGN | EXTENDED_SESSIONSECURITY
        m.extend_from_slice(&0x0008_8207u32.to_le_bytes());
        m.extend_from_slice(&[0u8; 16]); // empty domain + workstation buffers
        m
    }

    fn sec_buf(msg: &[u8], at: usize) -> Option<&[u8]> {
        let len = u16::from_le_bytes([*msg.get(at)?, *msg.get(at + 1)?]) as usize;
        let off = u32::from_le_bytes([
            *msg.get(at + 4)?,
            *msg.get(at + 5)?,
            *msg.get(at + 6)?,
            *msg.get(at + 7)?,
        ]) as usize;
        msg.get(off..off.checked_add(len)?)
    }

    /// Build the Type 3 (AUTHENTICATE) message from the server's Type 2.
    pub(super) fn type3_message(
        type2: &[u8],
        user: &str,
        domain: &str,
        pass: &str,
        client_challenge: [u8; 8],
        filetime: u64,
    ) -> Result<Vec<u8>> {
        if type2.len() < 32 || &type2[..8] != b"NTLMSSP\0" || type2[8..12] != 2u32.to_le_bytes() {
            return Err(anyhow!("NTLM: malformed Type 2 message"));
        }
        let server_flags = u32::from_le_bytes([type2[20], type2[21], type2[22], type2[23]]);
        let server_challenge = &type2[24..32];
        let target_info = sec_buf(type2, 40).unwrap_or(&[]);

        let v2 = ntowf_v2(user, domain, pass);
        let mut blob = vec![1u8, 1, 0, 0, 0, 0, 0, 0];
        blob.extend_from_slice(&filetime.to_le_bytes());
        blob.extend_from_slice(&client_challenge);
        blob.extend_from_slice(&[0u8; 4]);
        blob.extend_from_slice(target_info);
        blob.extend_from_slice(&[0u8; 4]);
        let mut proof_in = server_challenge.to_vec();
        proof_in.extend_from_slice(&blob);
        let mut nt_resp = hmac_md5(&v2, &proof_in).to_vec();
        nt_resp.extend_from_slice(&blob);
        let mut lm_in = server_challenge.to_vec();
        lm_in.extend_from_slice(&client_challenge);
        let mut lm_resp = hmac_md5(&v2, &lm_in).to_vec();
        lm_resp.extend_from_slice(&client_challenge);

        let (dom, usr, ws) = (utf16le(domain), utf16le(user), utf16le("FERRIC"));
        let flags = 0x0008_8205u32 | (server_flags & 0x0080_0000);
        let mut payload_off = 64u32;
        let mut header = b"NTLMSSP\0".to_vec();
        header.extend_from_slice(&3u32.to_le_bytes());
        let mut payload = Vec::new();
        for field in [&lm_resp, &nt_resp, &dom, &usr, &ws, &Vec::new()] {
            let len = field.len() as u16;
            header.extend_from_slice(&len.to_le_bytes());
            header.extend_from_slice(&len.to_le_bytes());
            header.extend_from_slice(&payload_off.to_le_bytes());
            payload.extend_from_slice(field);
            payload_off += u32::from(len);
        }
        header.extend_from_slice(&flags.to_le_bytes());
        header.extend_from_slice(&payload);
        Ok(header)
    }

    /// `DOMAIN\user` -> (domain, user); plain `user` has an empty domain.
    fn split_account(account: &str) -> (&str, &str) {
        account.split_once('\\').unwrap_or(("", account))
    }

    fn filetime_now() -> u64 {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        (secs + 11_644_473_600) * 10_000_000
    }

    pub(super) async fn handshake(
        client: &reqwest::Client,
        method: &str,
        url: &str,
        cookies: Option<&str>,
        extra_headers: Option<&HashMap<String, String>>,
        range: Option<&str>,
        user: &str,
        pass: &str,
    ) -> Result<reqwest::Response> {
        let m = reqwest::Method::from_str(method).unwrap_or(reqwest::Method::GET);
        let build = |auth: String| {
            let mut req = client.request(m.clone(), url);
            if let Some(r) = range {
                req = req.header("Range", r);
            }
            super::apply_request_headers(req, cookies, extra_headers).header("Authorization", auth)
        };
        let b64 = &base64::engine::general_purpose::STANDARD;
        let first = build(format!("NTLM {}", b64.encode(type1_message())))
            .send()
            .await?;
        let challenge = first
            .headers()
            .get_all("www-authenticate")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|v| v.trim().strip_prefix("NTLM ").map(str::trim))
            .map(str::to_string)
            .ok_or_else(|| anyhow!("NTLM: server did not send a challenge"))?;
        // Drain the 401 body so the keep-alive connection returns to the pool and
        // the Type 3 request is sent on the same connection.
        let _ = first.bytes().await;
        let type2 = b64
            .decode(challenge.as_bytes())
            .map_err(|_| anyhow!("NTLM: challenge is not valid base64"))?;
        let (domain, account) = split_account(user);
        let type3 = type3_message(
            &type2,
            account,
            domain,
            pass,
            rand::random::<[u8; 8]>(),
            filetime_now(),
        )?;
        build(format!("NTLM {}", b64.encode(type3)))
            .send()
            .await
            .map_err(anyhow::Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_validator_prefers_strong_etag_and_skips_weak_ones() {
        assert_eq!(
            range_validator(Some("\"abc\""), Some("lm")).as_deref(),
            Some("\"abc\"")
        );
        assert_eq!(
            range_validator(Some("W/\"abc\""), Some("Tue, 01 Jan 2030 00:00:00 GMT")).as_deref(),
            Some("Tue, 01 Jan 2030 00:00:00 GMT")
        );
        assert_eq!(range_validator(Some("W/\"abc\""), None), None);
        assert_eq!(range_validator(None, None), None);
    }

    #[test]
    fn md4_matches_rfc1320_vectors() {
        let hex = |d: [u8; 16]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(ntlm_auth::md4(b"")), "31d6cfe0d16ae931b73c59d7e0c089c0");
        assert_eq!(
            hex(ntlm_auth::md4(b"abc")),
            "a448017aaf21d8525fc10ae87aa6729d"
        );
    }

    #[test]
    fn ntowf_v2_matches_ms_nlmp_vector() {
        // MS-NLMP 4.2.4.1.1: User "User", Domain "Domain", Password "Password"
        let hex = |d: [u8; 16]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex(ntlm_auth::ntowf_v2("User", "Domain", "Password")),
            "0c868a403bfd7a93a3001ef22ef02e3f"
        );
    }

    #[test]
    fn ntlm_type3_proof_matches_ms_nlmp_vector() {
        // MS-NLMP 4.2.4: server challenge 0123456789abcdef, client challenge aa*8,
        // time 0, target info = NbDomainName "Domain" + NbComputerName "Server".
        let utf16 = |s: &str| {
            s.encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<u8>>()
        };
        let mut info = Vec::new();
        for (id, name) in [(2u16, "Domain"), (1u16, "Server")] {
            info.extend_from_slice(&id.to_le_bytes());
            info.extend_from_slice(&12u16.to_le_bytes());
            info.extend_from_slice(&utf16(name));
        }
        info.extend_from_slice(&[0u8; 4]);
        let mut t2 = b"NTLMSSP\0".to_vec();
        t2.extend_from_slice(&2u32.to_le_bytes());
        t2.extend_from_slice(&[0u8; 8]); // target name buffer
        t2.extend_from_slice(&0xe281_8235u32.to_le_bytes());
        t2.extend_from_slice(&[0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
        t2.extend_from_slice(&[0u8; 8]);
        t2.extend_from_slice(&(info.len() as u16).to_le_bytes());
        t2.extend_from_slice(&(info.len() as u16).to_le_bytes());
        t2.extend_from_slice(&48u32.to_le_bytes());
        t2.extend_from_slice(&info);
        let t3 = ntlm_auth::type3_message(&t2, "User", "Domain", "Password", [0xaa; 8], 0).unwrap();
        // NT response sits after the 64-byte header + 24-byte LM response; first 16 bytes = NTProofStr.
        let proof: String = t3[88..104].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(proof, "68cd0ab851e51c96aabc927bebef6a1c");
    }

    #[test]
    fn zero_size_splits_to_no_chunks() {
        assert!(split_into_chunks(0, 4).is_empty());
    }

    #[test]
    fn split_chunks_cover_file_once() {
        let chunks = split_into_chunks(10, 3);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].byte_start, 0);
        assert_eq!(chunks[2].byte_end, 9);
        assert_eq!(
            chunks
                .iter()
                .map(|c| c.byte_end - c.byte_start + 1)
                .sum::<u64>(),
            10
        );
    }

    #[test]
    fn plan_pieces_follow_the_initial_layout() {
        // IDM, 20 MiB file: starts 0, 233,600, ~1/2 and ~3/4 of the rest
        // (observed 10,610,751 and 15,791,135; ours are within 1 % of those).
        let size = 20 * 1_048_576;
        let starts: Vec<u64> = plan_pieces(size, 8).iter().map(|p| p.byte_start).collect();
        assert_eq!(starts[..2], [0, 233_600]);
        assert!(starts[2].abs_diff(10_610_751) < size / 100, "{starts:?}");
        assert!(starts[3].abs_diff(15_791_135) < size / 100, "{starts:?}");
    }

    #[test]
    fn plan_pieces_cover_the_file_with_four_pieces() {
        let pieces = plan_pieces(40 * 1_048_576, 8);
        assert_eq!(pieces.len(), 4);
        assert_eq!(pieces[0].byte_start, 0);
        assert_eq!(pieces.last().unwrap().byte_end, 40 * 1_048_576 - 1);
        let covered: u64 = pieces.iter().map(|p| p.byte_end - p.byte_start + 1).sum();
        assert_eq!(covered, 40 * 1_048_576);
    }

    #[test]
    fn plan_pieces_never_exceed_the_connection_cap_or_the_grain() {
        assert_eq!(plan_pieces(40 * 1_048_576, 2).len(), 2);
        assert_eq!(plan_pieces(40 * 1_048_576, 1).len(), 1);
        // 500,000 bytes holds only two grains.
        assert_eq!(plan_pieces(500_000, 8).len(), 2);
    }

    #[test]
    fn plan_pieces_single_piece_for_tiny_file() {
        let pieces = plan_pieces(100, 4);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].byte_end, 99);
    }

    #[test]
    fn content_disposition_filename_prefers_utf8_star() {
        let header =
            "attachment; filename=\"token\"; filename*=UTF-8''Dustwind.Resistance-SteamRIP.com.rar";
        assert_eq!(
            filename_from_content_disposition(Some(header)).as_deref(),
            Some("Dustwind.Resistance-SteamRIP.com.rar")
        );
    }

    #[test]
    fn planned_chunk_count_respects_connection_cap() {
        // IDM opens up to 8 connections for anything worth splitting.
        assert_eq!(planned_chunk_count(1_100_000, 32), 8);
        assert_eq!(planned_chunk_count(1_100_000, 1), 1);
        assert_eq!(planned_chunk_count(200_000_000, 6), 6);
        assert_eq!(planned_chunk_count(200_000_000, 0), 1);
    }

    #[test]
    fn make_hyper_client_builds_with_http2() {
        make_hyper_client().expect("HTTP/2-capable hyper client should build");
    }

    #[test]
    fn build_sources_dedupes_and_keeps_primary_first() {
        let sources = build_sources(
            "https://a.example/file",
            &[
                "https://b.example/file".into(),
                "https://a.example/file".into(),
                "https://c.example/file".into(),
                String::new(),
            ],
        );
        assert_eq!(
            sources,
            vec![
                "https://a.example/file",
                "https://b.example/file",
                "https://c.example/file",
            ]
        );
    }

    #[test]
    fn assign_source_round_robin() {
        assert_eq!(assign_source_index(0, 3), 0);
        assert_eq!(assign_source_index(1, 3), 1);
        assert_eq!(assign_source_index(2, 3), 2);
        assert_eq!(assign_source_index(3, 3), 0);
    }

    #[test]
    fn mirror_set_redistributes_after_failure() {
        let set = MirrorSet::new(vec![
            "https://a.example/f".into(),
            "https://b.example/f".into(),
            "https://c.example/f".into(),
        ]);
        let (i0, u0) = set.pick(0).unwrap();
        assert_eq!(i0, 0);
        assert_eq!(u0, "https://a.example/f");

        set.mark_dead(0);
        assert_eq!(set.live_count(), 2);

        // chunk 0 among survivors → first live = b
        let (i, u) = set.pick(0).unwrap();
        assert_eq!(i, 1);
        assert_eq!(u, "https://b.example/f");

        set.mark_dead(1);
        set.mark_dead(2);
        assert!(set.pick(0).is_none());
    }

    #[test]
    fn parse_content_digest_base64_sha256() {
        use sha2::{Digest, Sha256};
        let data = b"hello digest";
        let hash = Sha256::digest(data);
        let b64 = base64::engine::general_purpose::STANDARD.encode(hash);
        let header = format!("sha-256=:{b64}:");
        let (alg, hex) = parse_content_digest(&header).unwrap();
        assert!(matches!(alg, DigestAlg::Sha256));
        assert_eq!(hex, format!("{:x}", Sha256::digest(data)));
    }
}

#[cfg(test)]
mod user_agent_tests {
    use super::*;

    fn ua_of(extra: Option<HashMap<String, String>>) -> Option<String> {
        let req = apply_request_headers(
            reqwest::Client::new().get("http://x.test/"),
            None,
            extra.as_ref(),
        )
        .build()
        .unwrap();
        req.headers()
            .get("user-agent")
            .map(|v| v.to_str().unwrap().to_string())
    }

    #[test]
    fn a_request_without_a_browser_user_agent_gets_the_stable_default() {
        assert_eq!(ua_of(None).as_deref(), Some(default_user_agent()));
        let mine = HashMap::from([("User-Agent".to_string(), "Browser/1".to_string())]);
        assert_eq!(
            ua_of(Some(mine)).as_deref(),
            Some("Browser/1"),
            "the browser's own value wins"
        );
    }
}

#[cfg(test)]
mod changed_file_tests {
    use super::*;

    fn headers(etag: Option<&str>, lm: Option<&str>) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        if let Some(e) = etag {
            h.insert(ETAG, e.parse().unwrap());
        }
        if let Some(l) = lm {
            h.insert(LAST_MODIFIED, l.parse().unwrap());
        }
        h
    }

    #[test]
    fn a_ranged_response_from_another_version_of_the_file_is_detected() {
        let started = Some("\"v1\"");
        assert!(
            !response_changed(started, &headers(Some("\"v1\""), None)),
            "same version"
        );
        assert!(
            response_changed(started, &headers(Some("\"v2\""), None)),
            "ETag changed"
        );
        assert!(
            !response_changed(started, &headers(None, None)),
            "no marker: nothing to compare"
        );
        assert!(
            !response_changed(None, &headers(Some("\"v2\""), None)),
            "download started without a marker"
        );
        let lm = Some("Wed, 01 Jan 2025 00:00:00 GMT");
        assert!(
            response_changed(lm, &headers(None, Some("Thu, 02 Jan 2025 00:00:00 GMT"))),
            "Last-Modified changed"
        );
    }
}

#[cfg(test)]
mod server_filename_tests {
    use super::*;

    #[test]
    fn server_names_keep_their_last_component_and_extension() {
        // illegal characters become '-', the rest of the name survives
        assert_eq!(
            clean_server_filename("a:b*c?d.txt").as_deref(),
            Some("a-b-c.d.txt")
        );
        assert_eq!(
            clean_server_filename("..\\..\\evil.txt").as_deref(),
            Some("evil.txt")
        );
        assert_eq!(
            clean_server_filename("../up/evil2.txt").as_deref(),
            Some("evil2.txt")
        );
        assert_eq!(
            clean_server_filename("report final.pdf").as_deref(),
            Some("report final.pdf")
        );
        assert_eq!(clean_server_filename("..").as_deref(), None);
        assert_eq!(clean_server_filename("   ").as_deref(), None);
        assert_eq!(
            filename_from_content_disposition(Some("attachment; filename=\"a:b*c?d.txt\""))
                .as_deref(),
            Some("a-b-c.d.txt")
        );
    }
}
