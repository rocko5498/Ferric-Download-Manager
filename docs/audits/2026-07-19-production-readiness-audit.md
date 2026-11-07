# Production-Readiness Audit — 2026-07-19

Auditor: Claude (principal-architect pass, repo-first + live verification).
Build under audit: branch `claude/great-easley-e50783` @ 8123d94 (+ this doc).
Daemon and UI both running from `.claude/worktrees/great-easley-e50783/target/debug/`.

Method: full repository traversal → three-way wiring reconciliation (API routes ↔
CommandBus qinvokables ↔ QML call sites) → live driving of the real app with real
mouse/keyboard input and screenshot evidence → failure-path injection (daemon kill,
invalid input, resize stress) → reconciliation against the prior knowledge base
(2026-07-18 production audit, 2026-07-16 architecture review + invariants, FQT-128/129/130
campaign records). Facts and recommendations are separated; every claim names its evidence.

---

## 1. Repository inventory (source of truth)

| Surface | Count | Notes |
|---|---|---|
| Crates | 8 | ferricd, ferric-qt, ferric-common, ferric-extract (58 extractor modules), ferric-host, ferric-cli, ferric-cdp, ferric-plugin-sdk |
| ferricd modules | 38 + db/14 | incl. ftp/sftp/webdav, dash/hls, av, scheduler, retry, speed, throttle, geoip, kill_switch, product_mode, import_qbt |
| HTTP API routes | 62 paths (76 method-handlers) | extracted from api.rs route table |
| CommandBus qinvokables | 85 | command_bus.rs bridge block |
| QML files | 41 | 23 dialogs, 7 workspaces, tables/panels/chrome |
| QML→bus call sites | 79 unique names | all 79 resolve to real qinvokables |
| Extension | background + popup + policy | Chrome MV3, native messaging |
| Migrations, openapi, plugins | present | ferric-hello sample plugin |

`todo!()`/`unimplemented!()` in workspace: **0**. Named stubs: NTLM proxy auth
(chunk.rs:976, cfg-gated), AV scan no-op under `#[cfg(test)]`, plugin host fns
(deliberate facade, plugins_enabled=0 default).

## 2. Wiring reconciliation (repo ↔ app)

**Zero phantom QML calls.** Every `bus.*` name in QML resolves to a real qinvokable —
the FQT-129 defect class is clean at HEAD (verified by extraction + diff, after
correcting a multiline-declaration extraction error that briefly showed 11 false
phantoms).

**Six qinvokables have no QML caller** (dead-from-UI code): `batch_downloads`,
`batch_torrents` (UI loops per-item instead), `get_diagnostic_json`,
`get_torrent_tags`, `set_torrent_tags` (superseded by `get_download_diagnostic`,
`add/remove_torrent_tag`), `set_torrent_category`.

**Six API routes unreachable from the UI layer** (no CommandBus reference):
`/downloads/{id}/confirm`, `/media/extract-login`, `/media/probe`, `/media/select`,
`/torrents/engine/status`, `/torrents/{id}/audit`. ferric-cli/ferric-cdp do not call them
either. Daemon-only / future surface.

**All 23 dialogs are instantiated and have reachable openers** (menu, toolbar,
context menu, or programmatic trigger). Menubar: File(3+Exit), Downloads(3),
View(5), Tools(1), Options(1), Help(1) — all verified open live (shots a01–a06).
Search workspace is deliberately hidden unless `pluginList.length > 0` (T2
hide-unplugged-surface); with plugins default-off it is invisible — by design.

## 3. Live verification performed this session (evidence in scratchpad shots)

| Area | Result | Evidence |
|---|---|---|
| All 6 menubar menus open | PASS | a01–a06 |
| Downloads context menu (8 items incl. both delete variants) | PASS | b02 |
| Torrents context menu (9 items incl. Queue submenu) | PASS | b03 |
| Context-sensitive toolbars per workspace | PASS | b02 vs a04 |
| Dark-mode live toggle | PASS | a13/a14 |
| Resize: rapid loop, maximized, restore | PASS — stable, no clipping | a15, a16 |
| Resize: 700×480 minimum | **FAIL — overlaps (F-3)** | a14 |
| Add URL wizard step 1→2, folder Browse present | PASS | b04/b05 |
| Invalid URL server rejection | PASS — 400 "invalid URL: relative URL without a base" | curl |
| Esc closes Add URL dialog | **FAIL (F-2)** | c01/c02 |
| Daemon-down detection ("Disconnected" + red dot) | PASS | c03 crop |
| Daemon-restart reconnection | **FAIL (F-1)** | c04/c05 crops |
| Daemon data integrity across 4 kill/restart cycles | PASS — 2 torrents, 30 downloads, 1 queue intact | API counts |
| Privacy toast (passed-but-not-anonymous) | PASS | 25-toast (earlier today) |
| Torrent policy dialog + Check + Save button color | PASS | 24-policy |
| `cargo test --workspace` | **407 passed, 0 failed** | task log |
| Gated clippy (correctness+suspicious) | clean | earlier today |
| Zero QML errors across all UI runs | PASS | 0-byte stderr logs |

## 4. Findings (this audit)

**F-1 [High][correctness][CONFIRMED live] UI never reconnects after daemon restart.**
progress_hub.rs: `run_hub()` re-reads the lock file per iteration, but the SSE-failure
path enters `run_poll_until_daemon_recovers(port,…)` with the port captured at entry;
the loop has no lock re-read and no attempt bound. ferricd binds 127.0.0.1:0 (fresh port
per start), so after any daemon crash/restart the poll spins on the dead port forever:
status bar Disconnected permanently, SSE/progress dead, while CommandBus calls (per-call
lock reads) keep working — a confusing half-broken state. Reproduced: killed + restarted
ferricd with UI open; still Disconnected after several minutes (c03/c04/c05).
User impact: any daemon crash requires a manual app restart to restore live progress.
Fix chip filed (task_d12782da): bound the poll or re-read the lock inside it.

**F-2 [Medium][ux-keyboard][CONFIRMED live] Esc dead in ~13 of 23 dialogs.**
main.qml:1226 `Shortcut{sequence:"Escape"}` consumes Esc window-wide (Qt shortcut
priority beats item Keys), so AppDialog.qml:27 `Keys.onEscapePressed` never fires; the
shortcut's handler enumerates only 10 dialogs. Add URL/Add Torrent/Add Batch/Add RSS/
delete/set-location/stats/trackers/error/duplicate/exit/category dialogs cannot be
closed with Esc (c01/c02: wizard survives repeated Esc; the Options menu then opens over
the stuck modal). Same defect class as FQT-129: hand-maintained enumeration drifting
from the population. Fix chip filed (task_68aeb04f): generic topmost-dialog close.

**F-3 [Medium][layout][CONFIRMED live] Minimum-size overlaps at 700×480.**
(a) SideFilter status labels bleed into/over the status bar; (b) Columns/Speed Graph/Log
buttons overlap the Media/RSS workspace tabs. No minimumWidth/Height set on the Window,
and the tab row + action row share one line with no elision/priority rule. Recommend:
set sensible Window minimums (~860×560) or make the right-button cluster collapse.

**F-4 [Medium][theme][CONFIRMED live] Dark-mode menubar unreadable.**
Native-styled MenuBar keeps near-black text on the dark theme (a14/a16). Accepted in
R3 as "functional"; at production bar it is a first-screen defect for every dark-mode
user. Recommend styling the QtQuick.Controls MenuBar delegates from Theme.

**F-5 [Low][ux] URL validation only at final Add.**
"not-a-valid-url" sails through wizard step 1→2 (b05); rejection arrives only from the
daemon after the user has filled step 2 (message itself is clear). Recommend a cheap
client-side scheme check on Next.

**F-6 [Info][dead-code] The 6 orphaned qinvokables + 6 daemon-only routes** (§2).
Recommend: delete the superseded trio, decide batch-vs-loop intentionally, and mark
daemon-only routes in openapi as headless surface.

**F-7 [Env][machine] R: drive hit 0 bytes free mid-audit**, killing the first workspace
test run (`os error 112`) — main-repo `target/` alone was 64.7 GB. Freed 56 GB via
`cargo clean` on the main repo (regenerable; the running daemon/UI use the worktree
target, unaffected; post-merge rebuild was already required). Precedent: 16 GB RAM
commit exhaustion in HANDOFF. This box needs headroom monitoring for builds; not a
product defect.

**Retracted during verification:** "400 error body is empty" — the body carries a clear
message; the empty read was a PowerShell StreamReader artifact. (Kept here per the
report-what-was-wrong rule.)

## 5. Inherited evidence (verified previously, not re-driven this session)

Accepted on the strength of recorded evidence + unchanged code paths; build drift risk
noted where the verifying build predates HEAD:

- **FQT-128 campaign** (2026-07-18): every toolbar button, menu, dialog, context menu
  exercised in light+dark, 45+ screenshots, 3 defects found+fixed.
- **FQT-129** (2026-07-18): six end-to-end workflows live: HTTP download to COMPLETED,
  magnet probe→stage→delete, .torrent probe→stage→delete, RSS add, grabber crawl,
  media extract (YouTube real formats).
- **FQT-130/131** (2026-07-19): delete-files-from-disk full UI path; narrow-panel
  two-row layout; folder pickers.
- **2026-07-18 audit**: auth on all routes incl. /events; validate_url at 30+ sites;
  safe_filename at all 4 creation sites; ferric-host extension-ID allowlist; invariant
  #10 privacy defaults; CI green (6e5eb22).
- **Golden-path matrix** (2026-07-18): torrent stage/start/pause/resume/restart-recover,
  stale-lock respawn, empty-table regression — all green columns.
- **This session (earlier)**: privacy-check warnings (3 daemon states via API + toast via
  real clicks), engine gate unchanged, timeout tiering black-hole test.

## 6. Explicitly UNVERIFIED (with justification)

- **FTP/SFTP/WebDAV, DASH/HLS, AV scan, post-download actions, scheduler, retry,
  speed/throttle internals** — unit-tested (part of the 407) but never live-driven;
  no test servers/media sources in this environment.
- **Chrome extension E2E + native messaging chain** — requires extension install
  ceremony in a real Chrome profile; not performed. ferric-host ID validation verified
  at code level (2026-07-18).
- **ferric-cdp, ferric-cli** — compile and are tested in-workspace; no live exercise.
- **Import qBittorrent live** — no qB BT_backup on this machine; code path reviewed,
  timeout carve-out added today.
- **High-DPI, multi-monitor, non-100% scaling** — single-monitor 1920×1080 env.
- **Full keyboard-shortcut matrix** (Ctrl+O/Ctrl+L/Space etc.) — present in code;
  only Esc and Alt-mnemonics live-driven (Esc found broken, F-2).
- **Prefs dialog contents on HEAD build** — open blocked live by F-2's stuck modal in
  the scripted run; verified on 2026-07-18 build (FQT-128). Known open item regardless:
  theme choice does not persist across restarts (HANDOFF FQT-128 follow-up).
- **Media download/playlist full pipeline on HEAD** — extract verified live (FQT-129);
  download/merge not re-driven today.
- **Long-haul soak (memory/FD leaks over days)** — out of session scope; no leak
  evidence in today's multi-hour run.

## 7. Verdict

**Not production-ready yet — but close, and the gap is enumerable.**

Blockers before a public release (in order):
1. **F-1** daemon-restart reconnection (breaks the crash-recovery story; chip filed).
2. **AppHang architecture risk** (known, top of HANDOFF): 68 UI-thread `block_on`
   sites — today's timeout tiering bounds the freeze (4s interactive / 20s ops) but the
   async-bus rewrite remains the real fix.
3. **F-2 + F-3 + F-4** keyboard close, min-size overlaps, dark menubar — first-hour
   polish defects a reviewer will hit.
4. Theme persistence + shipping-default decision (light vs dark) — owner call pending.

Foundations that are genuinely solid, with evidence: 407/407 tests, CI green baseline,
auth-gated localhost API, privacy fail-closed gates + honest warnings (as of today),
data integrity through repeated daemon kills, zero QML errors across every run, zero
phantom wiring, clean dependency-audit baseline (2026-07-18).

Register updates: F-1/F-2 filed as chips; F-3/F-4/F-5/F-6 recorded here + HANDOFF.
