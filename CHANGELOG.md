# Changelog

## 1.0.0-rc.3

### New
- **Starts while the info dialog is open**: a browser capture begins downloading as soon as it is probed, at a free path. Start keeps it, a changed name or folder moves the file when the transfer ends, Download Later stops it, Cancel deletes what was fetched. Off with "Start downloading while the Download File Info dialog is open" in Preferences. Merge downloads and manifests are not started early.
- **Download limits**: stop all downloads after N MB within 1-23 hours (Preferences > Downloads).
- **PAC proxy**: an automatic proxy configuration script address next to a plain proxy server field (Preferences > Downloads). PAC runs in a built-in JavaScript engine; `dnsResolve` only knows literal IP addresses.
- **Site Grabber**: "Explore only", "Never open" patterns (logout pages), "Don't explore parent directories", "Levels on other sites", and saved projects and templates (settings plus found files).
- **Duplicates**: a new download never takes the path of an existing file or another listed download; it gets a numbered name (`file_1.zip`). A Preferences option replaces the existing file instead.
- **Extension**: a never-capture list (CSS, JS, JSON, images) and an internal-path rule, so page furniture stays with the browser.
- Sounds: when all downloads finish, when each file finishes, and a different one when a download fails (Preferences > General > Notifications).
- **Download Now** in the row menu: start a queued download past the queue's connection limit.
- **Grabber**: "Update project" explores a saved project again and reports what is new or gone; "Download everything found" explores and downloads in one step.
- **PAC** can optionally use the script address from Windows' Internet settings.
- A rename chosen in the info dialog while the download is still running is kept in the database, so it survives a restart; a proxy chosen there restarts the running download.
- GitHub workflows are manual-start only; `scripts/ci-local.ps1` is the check to run before pushing.

### Fixed
- A late "still downloading" database write could undo a finished piece after a rate-limit (429) fallback, so about 3 in 10 such downloads ended with "download finished with incomplete chunks".
- The Grabber treated another port on the same host as the same site.
- Resume: the log now says why a download restarts (size, content or date changed on the server).

### Local checks
- `scripts/ci-local.ps1` runs every check the GitHub workflows run (fmt, clippy, tests, Qt check, QML wire audit, extension build and tests, cargo-audit, cargo-deny) on your own machine. The gitleaks scan and the Linux test run are not covered.

## 1.0.0-rc.2

- The main window's close button really quits when the tray icon is hidden or "close to tray" is off (it always hid the window before, which looked like a dead button). "Close to tray" now defaults to on in Preferences, matching the behaviour.
- Failed downloads show the whole cause, e.g. "error connecting to socks proxy: SOCKS error: credentials not accepted" instead of a bare "error sending request".
- New option: play a sound when all downloads finish.

## 1.0.0-rc.1

First release candidate. Windows x64 installer package (`Ferric-win64.zip`), per-user install, no administrator rights.

### Speed
- Adaptive connection count on files of 32 MB and up: after the usual 4 then 8 connections, more are added (8, 12, 18, 24) while each batch still raises the measured speed; it stops at a plateau or when the server refuses an extra connection. Off when a per-host cap is set, when a site rule caps the host, or with `adaptive_connections=0`.
- A 502/503/504 answer to a ranged request is retried with back-off instead of dropping the file to one stream (server allowing 10 connections: 129 s -> 20 s in a local test).
- Connections no longer wait for the disk: a job-level checkpointer flushes every 2 s and then saves progress, so a saved offset is never ahead of the data. Byte-exact after pause/resume and after killing the daemon mid-download.
- Progress and speed come from a live view twice a second; the end of a download no longer waits out helper tasks.
- Speed limits (global and per download) now apply to running downloads; the limiter respects a limit within about 10%.
- Connections take turns over a host's several IP addresses.
- Media merge downloads (separate video + audio, e.g. YouTube): both streams now download at once over several 4 MB ranged connections instead of one request after the other.

### Familiar download-manager behaviour
- Queues: weekday schedule, start-on-startup, periodic synchronization, per-queue retries / open-file / when-done, Scheduler dialog switching between queues, Start/Stop queue toolbar buttons and menus, "Move to queue", queue picker on Download Later, default (disabled) Synchronization queue.
- Login prompt on HTTP 401 and proxy login prompt on 407, "save password" into the sites-logins list; SOCKS5 sends host names to the proxy.
- Find with file-name / description / site scopes, Match case, Whole string; View > Arrange files (11 sort keys), Hide categories, toolbar size, tray icon and font.
- A conservative default capture list; optional "set file date from server", "only add to queue", "ignore file time changes when resuming".
- Download complete window, drop target, export / import, when-done exit / shut down.

### Fixes
- Progress and the Size column now show correct values for finished downloads from servers that do not announce a length.
- Setting changes in Preferences refresh the interface at once; the proxy setting applies without a restart.
- Console windows no longer flash at logon; the app starts hidden with `--autostart`.

### Known limits
- Not code-signed (SmartScreen warns). No automatic updates.
- The browser extension has to be loaded unpacked. An off-store extension offered through the registry is listed by Brave/Chrome but cannot be switched on, so the installer does not register it (`ferric-host --install --offer-extension` does, for a future store listing).
- Firefox add-on unsigned (temporary add-on).
- Not built: download quota, F3 Find Next, "start while the info dialog is open", sounds, dial-up.
- A proxy login with a wrong SOCKS5 password fails with a generic "error sending request".
