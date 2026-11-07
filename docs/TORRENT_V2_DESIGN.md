# Torrent V2 Design

Torrent support is a candidate V2 feature for Ferric. It must be treated as a separate protocol subsystem, not as another flavor of HTTP chunk downloading.

## Product Rule

Ferric supports user-provided torrents and magnets as a general download workstation feature. The UI must not encourage infringement, bypass access controls, or seed unexpectedly.

## ADR-047: Engine Boundary

Decision: integrate a mature Rust torrent engine behind a narrow `TorrentEngine` adapter, with an isolated sidecar retained as the fallback if dependency audit or runtime behavior is not acceptable.

Current candidate: `librqbit`, because it exposes a library API, is used by the `rqbit` client, and already covers the hard peer/session work better than a hand-rolled engine.

Rejected for V2:
- Hand-rolling BitTorrent in `ferricd`.
- Mixing torrent sessions with `DownloadJob`, `ChunkWorker`, or browser credential logic.
- Starting with an always-on sidecar before proving library embedding is unsafe.

Why this boundary exists:
- Torrent is a long-lived distributed state machine: metadata, trackers, DHT, peer choke/unchoke, piece hashes, file mapping, disk IO, NAT, and seeding policy all interact.
- The failure mode is not only "download failed"; it can be privacy exposure, silent upload, wrong file writes, path traversal, high file descriptor usage, or disk pressure.
- A narrow adapter lets Ferric own product/security policy while the engine owns protocol details.

Proposed modules:
- `torrent`: intake/probe, policy validation, DTOs, and API handlers.
- `torrent_engine`: library or sidecar adapter implementing a common trait.
- `TorrentManager`: actor owning torrent sessions, lifecycle, and event fanout.
- `torrent_db`: torrents, files, pieces, trackers, peers, sessions, and limits.
- `TorrentSecurityPolicy`: save root, file selection, seeding, proxy, DHT/PEX/LSD, blocklist.

## Initial Implementation Slice

Implemented now:
- Magnet/.torrent intake DTOs in `ferric-common`.
- `POST /torrents/probe` in `ferricd`.
- Tauri `probe_torrent` command.
- UI "Torrent intake" modal.
- Magnet validation for `xt=urn:btih:<hash>`, display name, and tracker extraction.
- `.torrent` metadata inspection for single-file and multi-file torrents.
- File count and total size surfaced in the probe response.
- Canonical BitTorrent v1 info-hash computed from the exact raw bencoded `info` dictionary bytes.
- Pre-engine path security checks for traversal, absolute-path markers, empty path components, separators inside path components, and Windows reserved device names.
- Warnings for IP exposure, opt-in seeding, and path validation.

Not implemented yet:
- No peer networking.
- No torrent database tables.
- No file selection tree.
- No DHT/PEX/LSD engine controls.
- No seeding/upload.
- No BitTorrent v2 merkle root support yet.

## Security / Privacy Model

Must-have controls before engine start:
- Explicit user action for `.torrent` or magnet.
- Show public-IP visibility warning before first torrent start.
- No silent seeding. Default seed behavior is `stop after download`.
- Ratio limit and seed time limit.
- Per-torrent file selection before writing.
- Block path traversal, absolute paths, `..`, Windows device names, and mixed separator tricks inside torrent file lists.
- Refuse symlink tricks and non-file overwrite hazards.
- Disk quota and free-space check before allocation.
- Optional proxy/VPN binding, with warning if not active.
- DHT, PEX, and LSD toggles.
- Tracker/peer blocklist support.
- Optional encryption setting where supported.
- AV scan hook after completion.

Default policy:
- DHT: off until privacy warning is accepted.
- PEX: off until privacy warning is accepted.
- LSD: off by default on untrusted networks.
- Seeding: off after download unless user opts into ratio/time limits.
- Upload limit: conservative default when seeding is enabled.
- Save root: user-selected directory only; no implicit writes outside it.

## UI Model

Torrent intake:
- Magnet or `.torrent` path input.
- Parsed name, info hash, tracker count, and warnings.
- Disabled/gated "start" action until engine/security checklist is complete.

Torrent row:
- Name, total size, downloaded, uploaded, down speed, up speed, ETA, ratio, status.

Torrent detail tabs:
- Files: file tree, selected files, priority, progress.
- Peers: peer count, connected peers, client, encrypted/plain state, privacy warning.
- Trackers: tracker URL, status, scrape/announce result, last error.
- Pieces: piece map, availability, hash failures.
- Security: DHT/PEX/LSD, proxy/VPN status, seeding limit, blocklist status.

## API / Data Model Sketch

Tables:
- `torrents`: id, info_hash, name, save_root, status, total_size, downloaded, uploaded, ratio, created_at, completed_at.
- `torrent_files`: id, torrent_id, path, size, selected, priority, downloaded.
- `torrent_trackers`: id, torrent_id, url, status, last_error.
- `torrent_peers`: torrent_id, peer_id_hash, addr_hash, client, downloaded, uploaded, state.
- `torrent_pieces`: torrent_id, piece_index, size, status, availability.

Commands:
- `ProbeTorrent`
- `AddTorrentFile`
- `AddMagnet`
- `StartTorrent`
- `PauseTorrent`
- `RemoveTorrent`
- `SetTorrentFiles`
- `SetTorrentLimits`
- `SetTorrentPrivacy`

## Why Torrent Clients Fail

Common implementation traps:
- Treating pieces as files. Pieces cross file boundaries, so naive file writes corrupt multi-file torrents or bypass selected-file policy.
- Accepting torrent paths too early. Malicious metadata can attempt traversal, absolute paths, reserved device names, or symlink overwrite attacks.
- Starting DHT/PEX/LSD by default. Discovery features improve availability but increase privacy exposure.
- Silent seeding. Users experience "download complete" while the app keeps uploading.
- No backpressure. Peer connections, disk writes, hashing, and UI updates can overload the runtime if not bounded.
- Weak resume model. Torrents require piece verification, not just byte-count trust.
- Tracker-only assumptions. Magnet availability may depend on DHT or peers; tracker failure is not always fatal.
- Blocking UI on metadata fetch. Magnet metadata can take time; UI must show pending state without freezing.

## Required Tests Before Engine Wiring

- Magnet parser accepts valid v1 hex and base32 btih hashes.
- Magnet parser rejects missing or malformed info hashes.
- Tracker parser ignores unsafe URL schemes.
- `.torrent` metadata probe returns exact v1 SHA-1 info-hash. Implemented for local metadata intake.
- `.torrent` metadata parser rejects path traversal and absolute paths. Implemented for intake probe.
- File selection cannot write unselected files.
- Save-root containment works on Windows and Linux path separators. Initial component validation is implemented; final save-root canonicalization still belongs with engine/file-selection wiring.
- Default seeding is disabled after completion.
- Ratio/time limits stop seeding.
- Proxy/VPN warning appears before public peer discovery.
- DHT/PEX/LSD defaults are privacy-safe.
- Large torrent metadata does not freeze UI or allocate unbounded memory.
