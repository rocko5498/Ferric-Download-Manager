# Ferric — Roadmap

> Goal: win on substance through a combination of features no single download manager ships. Privacy as a threat-model dashboard, not a checkbox.

## Shipped ✓

| # | Feature |
|---|---------|
| 001-010 | Core: ferric-common, ferric-host, ferricd DB/IPC/API, downloader engine, security, extension, Tauri UI, CI, clippy |
| 011-019 | Queue engine, scheduler time windows, bandwidth throttle, priority UI |
| 020-021 | Site rules engine (per-host connection learning) |
| 022-025 | Code review, QA tests, checksum verify, crash recovery |
| 026-029 | Notifications/tray, autostart, native messaging registration, installers |
| 030-031 | Icons/branding, web store prep |
| 032-038 | Hyper chunk workers, adaptive scaling, HLS/DASH, content-script POST capture, dynamic segmentation, tail-splitting |
| 043-044 | Capture policy ecosystem, capture settings sync |
| 045-046 | Security hardening, media intelligence |
| 047-060 | Full torrent stack: design, metadata security, info-hash, manager+DB+UI, privacy gate, librqbit sidecar, proxy/VPN, file tree selection, privacy/fingerprint hardening, IP blocklist, AV scan hook, V2 Merkle, DHT/PEX/LSD wiring, diagnostics engine, HLS/DASH track selection |
| 064-069 | Refresh download address, Digest auth, Telemetry UI, Firefox/Edge expansion, Context menu Download All, Site grabber |
| 070-074 | V1.5 Features: CDP browser profile (ferric-cdp implementation) + Enterprise policy files, NTLM auth (Windows SSPI), Multi-depth site grabber BFS |

---

## Active backlog: None — V1 feature-complete

All tasks 001–069 shipped and verified in code. `cargo check --workspace` passes.

---

## V1.5 backlog (future, not started)

None. All scheduled V1.5 features (CDP, NTLM, Multi-depth BFS) have been completed.


## Effort legend
- S = < 2hr  M = half-day  L = 1-2 days  XL = 1 week+

---

## Differentiators

1. **Cross-session per-host connection learning** — no competitor does this
2. **Deep per-connection telemetry surface** — live segment map, diagnostic classification
3. **Torrent + HTTP unified** — one app, one queue, one scheduler
4. **Privacy as threat model** — DHT node rotation, fingerprint scrubbing, proxy-first design
5. **Cross-platform** (Windows + Linux)
6. **Open, no license nag**

---

## Not doing (explicit YAGNI)
- FTP support (declining protocol, low ROI)
- CDP browser automation before browser expansion (067) ships
- POST capture via content script (browser security model makes this fragile)
