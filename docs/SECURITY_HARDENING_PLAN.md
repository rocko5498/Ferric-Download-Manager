# Security Hardening Plan

This plan turns Ferric into a trustworthy official-build product while keeping open-source flexibility.

## 1. Official Build Signing

Goal: users can distinguish official builds from modified local builds.

Work items:
- Sign Windows installer and binaries.
- Sign Linux release artifacts where practical, at minimum publish checksums and signatures.
- Publish release checksums from CI.
- Add an "Official build" indicator in Settings/About when signature/build metadata validates.
- Add release provenance metadata: version, commit SHA, build profile, extension IDs, and signing status.

Acceptance:
- Official installer/binaries include verifiable signature or signed checksum.
- App can show build channel: `official`, `debug`, `local`, or `unknown`.

## 2. Unofficial / Debug Build Warning

Goal: avoid users mistaking local/debug builds for hardened production builds.

Work items:
- Add build metadata exposed from Tauri command: channel, commit, profile, configured extension IDs present/missing.
- UI warning banner in Settings/About for debug/local builds.
- Warn if production build lacks `FERRIC_EXTENSION_ID` or `FERRIC_FIREFOX_EXTENSION_ID`.

Acceptance:
- Debug build clearly says it trusts development extension IDs.
- Release build without production IDs is visibly unsafe/unofficial.

## 3. Extension Host Production ID Pinning

Current status:
- `ferric-host` now allows hardcoded dev IDs only in debug builds.
- Release builds only trust `FERRIC_EXTENSION_ID` / `FERRIC_FIREFOX_EXTENSION_ID`.
- Tagged release CI sets `FERRIC_OFFICIAL_BUILD=1`, requires both production IDs, and no longer falls back to development IDs.
- `ferric-host` and `ferric-ui` build scripts fail official builds when either production ID is missing.

Remaining work:
- Decide final Firefox ID, preferably `ferric@ferric.app` or another controlled domain-style ID.
- Keep support for Chrome/Edge IDs separate from Firefox IDs.

Acceptance:
- Official release cannot accidentally ship trusting unpacked dev extension IDs.

## 4. API Auth / Token Review

Current design:
- Daemon API binds to localhost.
- Random port and 32-byte token are written to lock file.
- UI reads lock file and sends token.

Review checklist:
- Lock file permissions should be user-only where the OS allows it. Current status: Unix uses `0600`; Windows removes inheritance and grants current user full control through `icacls`.
- Token must rotate on daemon start. Current status: daemon generates a fresh 32-byte random token at startup.
- API must reject missing/incorrect token on every non-health endpoint. Current status: middleware guards the router, including `/health` and SSE.
- No API endpoint should accept remote hosts or wildcard bind. Current status: daemon binds `127.0.0.1:0`.
- SSE/progress endpoint must require the same token. Current status: middleware covers `/events`.
- Errors must not leak cookies, auth headers, or full POST bodies. Current status: URL parse diagnostics are redacted; transient credentials are not logged.

Acceptance:
- Test proves protected endpoints reject missing/wrong token.
- Lock file content is not logged except path only.

## 5. Cookie / Secret Lifecycle Audit

Hard rules:
- Never read Chrome/Firefox cookie databases directly.
- Never persist cookies after download completion.
- Never log cookies, auth headers, POST bodies, or signed URLs beyond what is necessary for user-visible URL identity.

Current notes:
- Captured cookies and headers are held in memory for confirmation/start.
- Older architecture text mentions encrypted cookie DB; actual current design should prefer transient in-memory credentials unless there is a strict resumability requirement.

Work items:
- Audit all logs for URL/header/cookie leakage. Current status: normal daemon logs no longer include raw captured URLs; invalid URL errors redact query/fragment/credentials.
- Redact query tokens in diagnostics where possible while keeping enough origin info. Current status: `security::redact_url()` preserves scheme/host/path and redacts query, fragment, username, and password.
- Ensure pending capture creds are dropped on cancel, delete, completion, and daemon shutdown. Current status: pending capture/start credentials are memory-only and dropped on cancel/remove/confirm-to-start; daemon shutdown drops memory by process exit.
- If credential persistence is ever reintroduced, use explicit user consent, encryption, TTL, and automatic deletion.

Acceptance:
- Tests or code review prove credentials are removed on terminal states.
- Diagnostic UI says whether credentials exist without displaying them.

## 6. Download Path / File Deletion Safety

Current status:
- `safe_filename()` blocks traversal and reserved names.
- Delete-with-file refuses symlinks and non-file paths.
- Delete-with-file treats a missing file as already deleted and removes only the Ferric DB record.
- Manager save-path construction rejects filename path components.

Remaining work:
- Prevent torrent/media extraction from writing outside selected root.
- For delete-with-file, consider showing the exact file path and requiring confirmation.
- Refuse deletion for paths not owned by a Ferric download record.
- Keep "remove from list" separate from "delete file too."

Acceptance:
- Tests cover traversal, symlink, directory, missing file, and recorded regular file deletion.
