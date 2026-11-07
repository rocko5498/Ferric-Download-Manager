# Ferric — Architecture Reference
# (by Ferric; internal codebase pet name: Ferric)

Compact reference. Full design rationale is in the original design session.

## System diagram
```
Chrome Extension (MV3, Plasmo + TypeScript)
  ├── Service worker (sw)       — captures downloads.onCreated, cancels, forwards
  ├── Content script            — pre-click intercept, blob URL hook
  └── Popup                     — status, settings
         │
         │  Native Messaging (Chrome protocol: 4-byte LE length + JSON, stdio)
         ▼
ferric-host (thin relay binary, sync Rust)
  └── validates extension ID from argv
         │
         │  IPC: postcard frames over named pipe (Win) / unix socket (Linux)
         ▼
ferricd (Tokio async daemon)
  ├── IPC server                — accepts ferric-host connections
  ├── DownloadManager actor     — owns all jobs, dispatches commands
  │     └── DownloadJob × N
  │           ├── ChunkPlanner  — HEAD → Accept-Ranges → split ranges
  │           ├── ChunkWorker × M (hyper direct, HTTP/1.1 Range requests)
  │           ├── DiskWriter    — pre-alloc, seek+write per chunk
  │           ├── SpeedTracker  — 5s sliding window
  │           └── RetryCoord    — exp backoff per chunk
  ├── Scheduler                 — queue time windows, priority ordering
  ├── BandwidthLimiter          — token bucket (governor), global + per-dl
  ├── axum HTTP API             — localhost only, random port, auth token
  └── SQLite (sqlx)             — downloads, chunks, cookies_enc, settings
         │
         │  axum HTTP (127.0.0.1:PORT, auth token in lock file)
         ▼
ferric-ui (Tauri v2) — DEPRECATED, archived
  └── Replaced by ferric-qt (see below)

ferric-qt (cxx-qt, official UI)
  ├── CommandBus (Rust)           — bridge to daemon HTTP API
  ├── QML frontend                — download list, torrent panel, preferences, overview
  ├── System tray                 — minimize, show, quit, notifications
  ├── ProgressHub                 — SSE event stream → live speed/status
  └── DownloadModel / TorrentModel — Qt list models backed by daemon JSON

Torrent flow (parallel to HTTP downloads):
ferric-qt → ferricd TorrentManager → librqbit sidecar (subprocess)
                                    └── HTTP API on random local port
                                    └── proxy: SOCKS5/HTTP via --socks-url arg
                                    └── DHT/PEX/LSD policy controlled by ferricd
```

## Crate responsibilities

### `ferric-cli` (binary, not in default-members)
Terminal companion to the daemon: `Cli`/`Cmd` subcommands (health, list, add, privacy, audit export). Reads daemon endpoint via the same lock-file convention. Not previously documented anywhere in this file.

### `ferric-common`
- All IPC message types (`DownloadRequest`, `ProgressEvent`, `DaemonCommand`, …)
- All DB model types (`DownloadRecord`, `ChunkRecord`, `QueueRecord`, `SiteRule`, …)
- `FerricError` enum (thiserror)
- Nothing that imports tokio or does I/O

### `ferric-host`
- Single binary, sync I/O only (no tokio)
- Reads Chrome native message from stdin, writes to stdout
- Connects to daemon socket, relays messages bidirectionally
- Validates extension ID from process argv before touching IPC
- Must start daemon if it's not running
- Supports `ferric-host --install [extension-id]` and `--uninstall` for per-user
  Chrome/Edge native messaging registration. Production builds can set
  `FERRIC_EXTENSION_ID`; dev builds fall back to the unpacked extension ID.

### `ferricd`
- `main.rs`           — single-instance check, tracing init, start IPC + API servers
- `manager.rs`        — DownloadManager: mpsc command channel, broadcast progress
- `job.rs`            — DownloadJob: owns one download's full lifecycle
- `chunk.rs`          — ChunkPlanner + ChunkWorker (hyper connections)
- `disk.rs`           — DiskWriter: preallocate + seek+write
- `throttle.rs`       — BandwidthLimiter (token bucket via governor)
- `retry.rs`          — exponential backoff with jitter
- `speed.rs`          — SpeedTracker: sliding window ETA + speed
- `scheduler.rs`      — Queue time windows, priority queue
- `db/`               — sqlx queries (downloads, chunks, settings, cookies, queues, site_rules, torrents, proxies, peer_bans, merge_jobs)
- `ipc.rs`            — IPC server (named pipe / unix socket)
- `api.rs`            — axum HTTP API for ferric-qt UI + ferric-ui (deprecated)
- `security.rs`       — URL validation, filename sanitization, SSRF prevention, DNS rebind defense
- `av.rs`             — optional AV scan hooks (Defender/ClamAV) post-download
- `torrent.rs`        — BitTorrent metadata parsing, V2 Merkle piece hash validation
- `torrent_engine.rs` — librqbit sidecar process manager, proxy wiring, DHT policy, blocklist
- `torrent_manager.rs`— TorrentManager actor (mirrors DownloadManager for torrents)
- `hls.rs`            — HLS manifest parser + sequential segment downloader
- `dash.rs`           — DASH MPD parser + adaptation set / track selector
- `media.rs`          — ffmpeg merge job runner for HLS/DASH multi-track output
- `grabber.rs`        — URL refresh flow (re-auth expired CDN links)
- `diagnostic.rs`     — download failure classifier, DiagnosticPolicy
- `diagnostics.rs`    — diagnostics output path + event file writer
- `autostart.rs`      — OS autostart registration (registry on Windows, systemd user unit on Linux)
- `rss_watch.rs`      — RSS feed polling, enclosure extraction, auto-download rules
- `dns.rs`            — CachingResolver: custom DNS resolver with TTL cache (feeds SSRF/DNS-rebind defense in `security.rs`)
- `ftp.rs`            — FTP protocol downloads (rustls-backed control connection, chunk planning/download)
- `webdav.rs`         — WebDAV protocol downloads (PROPFIND size discovery, segmented planning)
- `sftp.rs`           — SFTP protocol downloads (SSH key auth, segmented planning)
- `ffmpeg_sandbox.rs` — sandboxed ffmpeg execution (Windows Job Object confinement)
- `webhooks.rs`       — emits configured webhook events (e.g. download-done)
- `geoip.rs`          — MMDB peer GeoIP lookup + `EyesTier` (Five/Nine/Fourteen-Eyes jurisdiction classification) for torrent peers
- `postdownload.rs`   — yt-dlp-style sidecar writer: info.json, description, chapters.txt, thumbnail embed, subtitles
- `kill_switch.rs`    — live SOCKS5 proxy health probe; pauses active torrents if the configured proxy drops
- `privacy.rs`        — `anonymity_score()`: 0-100 weighted privacy score (proxy/DHT/seeding/blocklist/kill-switch) + dashboard

### `ferric-qt` (official UI)
- `src/command_bus.rs`  — cxx-qt bridge: Q_INVOKABLE methods → daemon HTTP API
- `src/progress_hub.rs` — SSE event stream → live speed/status Q properties
- `src/transfer_models.rs` — Qt list models (DownloadModel, TorrentModel) backed by daemon JSON
- `src/enablement.rs`  — pure functions: which UI actions are enabled per transfer status
- `src/daemon_session.rs` — lock file parse, health check, daemon lifecycle
- `src/single_instance.rs` — single-instance guard (shared memory on Windows)
- `src/commands.rs`     — `dispatch()`: command entry point
- `src/http_client.rs`  — shared reqwest client for CommandBus/TransferModels HTTP calls to daemon
- `qml/main.qml`       — full UI: menu bar, workspace tabs, download/torrent tables, preferences, dialogs

### `ferric-ui` (DEPRECATED — archived, replaced by ferric-qt)
- `src/main.rs`    — Tauri entry point, system tray
- `src/commands.rs`— Tauri commands that proxy to daemon HTTP API
- `ui/`            — Solid.js: DownloadList, SpeedGraph, CaptureDialog, QueueEditor

## IPC message types (verified against crates/ferric-common/src/messages.rs + actual call sites in ipc.rs/api.rs, 2026-07-16)

**Correction (2026-07-16, same day as first written)**: an earlier pass of this section documented `DaemonCommand` as the UI/API → Daemon command type based only on reading its definition. Tracing actual call sites (`find_referencing_symbols`) shows `DaemonCommand` is dead code — referenced exactly once in the whole graph, its own re-export in `lib.rs`. Nothing constructs or matches it. The real internal command vocabulary is `ManagerCommand` (`ferricd::manager`, NOT in ferric-common — daemon-internal only, 10 variants: `Add`, `Confirm`, `Pause`, `Resume`, `Cancel`, `Remove{delete_file}`, `SetPriority`, `SetSpeedLimit`, `UpdateUrl`, `PumpQueue`). Both the IPC path (`ipc.rs::dispatch`) and the HTTP API path (`api.rs` handlers) construct `ManagerCommand` values directly and send them over the same `mpsc<ManagerCommand>` channel into `DownloadManager` — `DaemonCommand` was never the bridge between them.

```
Extension → Host → Daemon (binary length-prefixed frames) — HostMessage:
  Ping
  DownloadReq(DownloadRequest)        — only entry point for extension-submitted downloads
  MediaCandidate(MediaCandidate)
  GetCapturePolicy
  SetCapturePolicy { settings }
  (NOTE: no pause/resume/cancel/etc. over this transport — HostMessage has no such variants.
   Extension-submitted downloads land as ManagerCommand::Add{ hold: true, .. }, i.e. inserted
   Paused and waiting for a Confirm from the capture dialog, not auto-started.)

Daemon → Host → Extension — DaemonMessage: Pong{version} | Reply(DaemonReply) | Ack | CapturePolicy{settings}
  DaemonReply: Accepted{id} | Error{id, message}

ferric-qt (HTTP API) → Daemon — ManagerCommand (daemon-internal, crates/ferricd/src/manager.rs):
  Add{req, save_dir_override, hold, audio_url?, subtitle_url?, subtitle_lang?, proxy_url?, reply_tx}
  Confirm{id, filename?, save_dir?, url?, audio_url?, subtitle_url?, subtitle_lang?, proxy_url?, reply_tx}
  Pause(id) | Resume(id) | Cancel(id) | Remove{id, delete_file} | SetPriority{id, priority}
  SetSpeedLimit{id, bps} | UpdateUrl{id, new_url, reply_tx} | PumpQueue (internal, queue-slot-freed trigger)

Daemon → UI (axum SSE) — ProgressEvent (serde tag = "event", snake_case), 7 variants, shared with ferric-common:
  Added          { record: DownloadRecord }
  Progress       { id, downloaded, total?, speed_bps, eta_secs?, status }
  Done           { id, path, checksum_ok? }
  Failed         { id, message }
  Removed        { id }
  MediaCandidate { candidate: MediaCandidate }
  TorrentUpdated { record: TorrentRecord }
```

**Architectural implication**: extension captures and ferric-qt/API-submitted downloads take genuinely different paths into the same actor — extension captures always `hold: true` (paused, needs explicit Confirm), API/UI adds always auto-start. This is a real, intentional behavioral asymmetry (see `ManagerCommand::Add` doc comment), not a bug — but it means `DaemonCommand`'s existence as dead, unused, never-constructed code in a shared crate is itself worth a cleanup decision: either wire it up for a real purpose or delete it.

## Chunk strategy
```
HEAD → has Accept-Ranges + Content-Length?
  Yes → split into N chunks (2 → ramp → max 16)
        adaptive: add 2 connections every 3s if speed growing, back off on 429
        per-site rule persisted to SQLite after first learning
  No  → single HTTP/1.1 stream, no chunking
Resume: SELECT incomplete chunks → Range: bytes=start+downloaded-end → continue
```

## Database tables
26 migrations as of 2026-07-16 (`migrations/001_initial.sql` .. `027_torrent_queue_position.sql`) — list below covers the tables backed by a `db/*.rs` module (per `db/mod.rs`'s own module list) plus the torrent sub-tables; not an exhaustive column-by-column schema reference, see the migration files for that.
```
downloads       — id, url, filename, save_path, status, total_size, downloaded, etag, referrer, …
chunks          — id, download_id, chunk_index, byte_start, byte_end, downloaded, status
cookies         — id, download_id, data_enc (AES-256-GCM), nonce, domain
settings        — key, value
queues          — id, name, max_concurrent, max_bandwidth, sched_start, sched_end
site_rules      — id, pattern (glob), rule_type, value
torrents        — id, info_hash, name, status, save_path, policy fields (dht/pex/lsd/seed_enabled, …),
                   queue_position (027, FQT-076/077), created_at, completed_at
torrent_files   — id, torrent_id, path, size, selected, priority, downloaded
torrent_trackers— id, torrent_id, url, status, last_error
torrent_peers   — id, torrent_id, addr_hash, client, downloaded, uploaded, state
torrent_pieces  — id, torrent_id, piece_index, size, status, availability
peer_bans       — id, peer_addr, reason, banned_at
proxies         — id, name, proxy_type, host, port, username?, password?, enabled
merge_jobs      — id, video_path, audio_path, output_path, state, message, created_at, completed_at
extract_meta    — media-extraction metadata cache (db/mod.rs module; not previously documented here)
audit           — download/torrent audit log backing GET /audit/export (not previously documented here)
```

## Security boundaries
- All binaries run as current user — no elevation
- Daemon API: localhost only, random port (written to lock file), 32-byte auth token
- Extension ID allowlist: compile-time `FERRIC_EXTENSION_ID` / `FERRIC_FIREFOX_EXTENSION_ID` in release; dev IDs only under `debug_assertions`
- `security::validate_url()`: http(s) + LAN file-transfer schemes; DNS resolve **fails closed** (private/metadata blocked)
- Secondary URLs (mirrors, audio/subtitle, HLS AES keys, grabber links, thumbnails) also validated
- `security::safe_filename()`: strips traversal, Windows reserved names, null bytes
- Download-scoped cookies: AES-256-GCM in SQLite while active, deleted on completion — **never** read browser cookie DBs from disk
- Private IP bypass: `FERRIC_ALLOW_PRIVATE_IPS=1` honored **only in debug builds**
- Portable mode: `portable` sentinel next to exe → `./data` for both `ferricd` and `ferric-ui`
- Torrent: no automatic seeding; LSD/PEX disabled by default; proxy kill-switch path
- `kill_switch.rs` probes the configured SOCKS5 proxy on an interval; on failure it pauses all active torrents rather than letting them fall back to a direct (deanonymizing) connection
- `privacy.rs` computes a live 0-100 anonymity score (proxy 50pts / DHT-off 15 / seeding-off 10 / blocklist 15 / kill-switch 10) with an A-F grade, surfaced in the QML overview
- `geoip.rs` classifies torrent peers by `EyesTier` (Five/Nine/Fourteen-Eyes surveillance-alliance jurisdiction) via local MMDB lookup — no external API calls per peer

## CommandBus serialization pattern (undocumented until 2026-07-16)
`ferric-qt/src/command_bus.rs` has ~80 `Q_INVOKABLE` methods that call the daemon HTTP API and return `Result<String, String>` — raw JSON text passed straight to QML, parsed there with `JSON.parse()`. Only two functions in `transfer_models.rs` (`fetch_torrent_rows`, `fetch_download_rows`) do strict typed deserialization via `resp.json::<Vec<T>>()`. This means a struct field missing `#[serde(default)]` (e.g. `TorrentRecord.tags`, historically) can hard-fail those two list-population paths against a stale daemon binary, while every other CommandBus call degrades gracefully (missing fields just read as `undefined` in QML). When adding a new field to `TorrentRecord`/`DownloadRecord`, add `#[serde(default)]` unless the field must be mandatory.

## MVP scope (implement first)
- `ferric-common`: types only
- `ferric-host`: relay, extension ID validation, daemon autostart
- `ferricd`: IPC server, DB layer, manager actor, basic chunk download (reqwest OK for MVP), pause/resume
- `ferric-ui`: download list, add-URL dialog, progress bars, system tray

## V1 additions (after MVP works)
- Hyper direct for chunk workers
- Adaptive connection scaling per-host
- Bandwidth throttling
- Queue scheduler
- Content script pre-click intercept
- Extension published to Chrome Web Store

## V1.5 (Power Mode)
- HLS/DASH detection and download
- Chrome Enterprise policy extension
- POST download capture via content script
- CDP-based controlled browser profile

## V2 (Media Intelligence - Task 076)
- `ferric-extract`: pure-Rust extractors for YouTube, Vimeo, Reddit, and generic HTML media tags.
- `POST /media/extract`: daemon endpoint with existing `security::validate_url()` SSRF gate.
- FormatPicker UI: shows extracted formats and queues the chosen CDN URL through existing downloads.
- Extracted formats feed the existing ChunkWorker path; no second download engine.
