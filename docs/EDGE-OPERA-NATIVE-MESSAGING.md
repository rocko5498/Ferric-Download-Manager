# Edge & Opera native messaging (Tasks 120–121)

## Edge (Task 120)

Ferric uses the same Chromium native messaging host name: `com.ferric.host`.

| Check | Status |
|-------|--------|
| Host manifest install path (Windows) | `native-messaging/` install scripts register for Chrome **and** Edge when present |
| Extension ID pin | Same allowlist pattern as Chrome (production ID in host manifest) |
| Daemon relay | `ferric-host` ↔ `ferricd` IPC unchanged |
| Manual verify | Load unpacked MV3 in Edge → capture a download → confirm tray/daemon accepts |

Edge store packaging: reuse Chrome MV3 build; submit to Microsoft Edge Add-ons with the same `externally_connectable` / host permissions as Chrome.

## Opera (Task 121)

Opera is Chromium-compatible for WebExtensions MV3.

| Check | Status |
|-------|--------|
| Manifest | Same as Chrome (`extension/build/chrome-mv3-prod`) |
| Native messaging | Host name `com.ferric.host` — register under Opera’s native messaging path if distinct on OS |
| Verify | Load extension in Opera developer mode; confirm `connectNative` does not error |

No separate Opera-only source tree is required for V1.

## Policy sync (Task 122)

Daemon → extension: `capture_policy` message on native port (already handled in `background/index.ts`).  
Extension → daemon: `SetCapturePolicy` + `sync_policy_to_daemon` from popup.

When SiteRules change in the daemon, UI/settings update should call the existing capture policy push so the extension reflects daemon state without restart.
