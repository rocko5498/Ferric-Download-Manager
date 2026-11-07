# Production Audit Notes — 2026-07-18 (Claude, principal-architect pass)

## REMEDIATION OUTCOME (same day, user-approved)
All three before-release blockers from the audit are CLOSED:
1. **CI green on HEAD** — run on 6e5eb22: all 5 jobs pass (Rust ubuntu+windows, ferric-qt Windows, extension, security audit). First green run in repo history. Took 3 rounds:
   - Round 1 (54c874f): ci.yml repaired (ferric-ui steps removed, ferric-qt isolated to Qt job, qtdeclarative unlisted from aqt modules, clippy gates correctness+suspicious), 22→0 warnings, plugin engine fixed (bytes-vs-pages + epoch ticker) with 5 new tests (398→403), PluginsDialog removed, release/ binaries + junk files untracked, 129 commits pushed.
   - Round 2 (e1c9f32): .gitattributes eol=lf (Windows runner CRLF checkout vs rustfmt newline_style=Unix); api.rs test_state TempDir leak fix (drop deleted SQLite DB under live pool — masked on Windows, SQLITE_CANTOPEN on Linux); ferric-qt job checks QT_ROOT_DIR (install-qt-action v4 dropped Qt6_DIR).
   - Round 3 (6e5eb22): 3 tests hard-coding Windows drive letters made platform-conditional (security save-path, torrent probe, engine save-root).
2. **Hygiene** — release/ (48 DLL/EXE) untracked, 5 junk files deleted, .gitignore + .gitattributes added. History rewrite for 794 MB .git deliberately deferred.
3. **Plugin surface decided** — engine actually fixed + tested (incl. hostile-loop timeout kill); dialog removed from UI since host fns remain stubs; API stays behind plugins_enabled=0.

Bonus finding from CI: Linux legs had NEVER run these tests — two real cross-platform test bugs (TempDir lifetime, drive-letter paths) surfaced and fixed.

## Session inventory
- Baseline: **no git tags exist** (`git tag` empty). Audit is of HEAD = 7f665d8. Regression pass scoped to recent history (last ~69 commits since 2026-07-16).
- Build/test: `cargo check --workspace`, `cargo test --workspace` (running in background at session start). HANDOFF claims 398/398 pass.
- Prior knowledge base (read this session): `docs/superpowers/reports/2026-07-16-architectural-invariants.md`, `2026-07-16-architecture-review-findings.md`, `2026-07-18-architecture-recovery-adr.md`.
- Tools: rtk wrappers, serena LSP available, no CI config checked yet.

## Risk list (Phase 1)
1. Localhost HTTP API surface (ferricd/api.rs): auth token, CORS policy vs "zero day-one exploits" constraint (malicious website → localhost).
2. ferric-host extension-ID validation — invariant #9 NEVER verified by prior reviews.
3. security.rs validate_url / safe_filename coverage at all entry points (invariant #7 CONVENTIONAL).
4. Torrent privacy fail-safes (invariant #10) — spot-check still holds at HEAD.
5. SQLite pool: transactions spanning awaits (invariant #15 open item).
6. Plugin sandbox (wasmtime 36) — host fn surface.
7. Serde wire compat (#5) — round-trip tests added at HEAD, verify they exist and bind.
8. Heavy QML churn at HEAD (39 files, Theme/AppDialog migration) — regression risk, smoke claims PASS.
9. Repo hygiene: junk files `,session_title`, `36)`, `All`, `QString` committed at HEAD (verified via git show --stat).

## Findings so far
- [Blocker][Verified][release-eng] CI red on all runs since ≥2026-07-02; latest run 2026-07-15 (sha 3a4e5d8). HEAD is 128 commits ahead of origin/main — never pushed, never CI'd. At HEAD, ci.yml `rust` job still builds `crates/ferric-ui/ui` which no longer exists (removed in 7f665d8), and clippy runs `-D warnings` while workspace emits ~22 warnings → HEAD cannot pass its own CI. Evidence: `gh run list` (8/8 FAIL), `git rev-list origin/main..HEAD --count` = 128, ci.yml:57-60, `git ls-files crates/ferric-ui` empty.
- [High][Verified][correctness, latent] plugins/engine.rs:113-114 — `memory.data().len()` (bytes) treated as pages, multiplied by 65536 → base index ~4 GiB into 64 KiB memory → guaranteed panic (caught by thread join → error). Plugin invoke can never succeed. Also engine.rs:57/94 sets epoch deadline but **no `increment_epoch` anywhere** → 30s timeout never enforced (matches dead-field warning SandboxLimits.timeout). Also `_invoke` declared `-> result_ptr` in doc but called with 0 results. MITIGATION: unreachable in production — `register_with_wasm` has zero callers; HTTP register stores manifest only; host fns http_fetch/store_get/store_set all return stubs. Plugin subsystem is a facade behind `plugins_enabled` (default off).
- [Medium][Verified][robustness] security.rs validate_url does synchronous `to_socket_addrs()` DNS inside async handlers (api.rs add_download etc.) — blocks tokio workers. Also check-time DNS ≠ fetch-time DNS → rebinding TOCTOU (Likely, standard gap).
- [Medium][Likely][security-hardening] ipc.rs:275-278 — named pipe created without `first_pipe_instance(true)`; local pipe-instance squatting possible (same-user threat model, low practical impact; tokio default reject_remote_clients mitigates remote).
- [Low][Verified][hygiene] Junk shell-artifact files tracked: `,session_title`, `36)`, `All`, `QString`, `u64` (0-byte).
- [Low→Medium][Verified][hygiene] `release/` with 48 tracked .dll/.exe (full Qt runtime + built ferric-qt.exe/ferricd.exe/ferric-host.exe); `.git` = 794 MB.
- [Low][Verified][hygiene] deny.toml stale ignores (tauri-era) → warnings; `cargo deny check advisories` = **ok** at HEAD locally.
- [Info][Verified] Invariant #9 (ferric-host validates extension ID before forwarding) NOW VERIFIED: validate_caller() at ferric-host/src/main.rs:90 before any forward; compile-time allowlist; dev IDs debug-only.
- [Info][Verified] Invariant #10 holds at HEAD: torrent_manager.rs:84-108 re-derives DHT/PEX/LSD from settings (default off), private forces+locks off; engine start refuses bare-IP when proxy required (line 265-268).
- [Info][Verified] Invariant #5 now test-enforced: models.rs:255 `download_record_deserializes_minimal_json` + torrent equivalent (old-shape fixtures).
- [Info][Verified] All API routes incl /health and /events behind auth middleware (api.rs:393); bind 127.0.0.1:0; token constant-time compared; no CORS layer (correct — custom header requirement blocks browser CSRF).
- [Info][Verified] Build: 0 errors/22 warnings; tests 398/398 pass (1.98s). validate_url at 30+ call sites; safe_filename at all 4 file-creation sites (api.rs:1888,2232; job.rs:690; manager.rs:1031).

## Empirical UI verification
- ui-smoke.ps1 re-run 2026-07-18 against freshly built ferric-qt.exe: **PASS** — window appears, 0 QML errors, all 6 workspaces render with content (white 0%, text 2%), against live daemon on port 54874 (29 downloads, 7 torrents). Caveat: the running daemon process predates the HEAD rebuild (UI-at-HEAD vs daemon-at-~Jul-17); daemon binary itself rebuilt clean this session.
- unsafe code: 4 real sites, all justified (disk.rs:73 Win32 free-space, libc::getuid ×3 on Linux paths, cxx-qt `unsafe extern "C++"` bridge). Workspace lints set `unsafe_code = "warn"`.
- unwrap/expect in ferricd: ~209 grep hits incl. test modules — NOT individually audited; recommend `clippy::unwrap_used` lint for production paths instead of manual review.

## Open / not done
- Not examined in depth: db/* internals beyond txn grep (3 `pool.begin()` sites), torrent_engine.rs, media extractors, ferric-cdp, ferric-extract, ftp/sftp/webdav, retry/scheduler/speed/throttle.

## Commands run
- `git tag` → empty. `git show --stat HEAD` → junk files. 69 commits since 07-16; 128 unpushed vs origin/main; diff = 332 files, +57,077/−23,327.
- `cargo check --workspace` 0 err/22 warn; `cargo test --workspace` 398 pass; `cargo deny check advisories` ok.
- `gh run list` 8/8 FAIL; last green: none in visible history (11 runs, all failure/cancelled).
