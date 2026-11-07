# Ferric Chrome Extension

Extension ID: `gepkkfjfgdcgelcnejpikdeanggknbjo` (the packed `Ferric.crx`, signed with the Ferric key; see `scripts/pack-extension.ps1`). Earlier unpacked builds used `bngipfinnjpcnebfchfbeinfppepibkb`, which the host still accepts.

## Build

```powershell
npm install
npm run build
```

Load this folder in Chrome dev mode:

```text
extension/build/chrome-mv3-prod
```

## Package for Chrome Web Store

After building, create the submission ZIP:

```powershell
npm run build
Compress-Archive -Path build/chrome-mv3-prod/* -DestinationPath ferric-extension-0.1.0.zip
```

On Linux/macOS:

```bash
npm run build
cd build/chrome-mv3-prod && zip -r ../../ferric-extension-0.1.0.zip .
```

The CI release workflow (`release.yml`) automates this — the ZIP is uploaded as a
release artifact alongside the desktop installers.

## Native Host

Build and install the native messaging host first:

```powershell
cargo build -p ferric-host
powershell -ExecutionPolicy Bypass -File ..\native-messaging\install-chrome.ps1
```

The install script registers `com.ferric.host` for the current Windows user.

## Manual Smoke Checks

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Choose "Load unpacked" and select `extension/build/chrome-mv3-prod`.
4. Confirm Ferric loads without errors and the extension ID is `bngipfinnjpcnebfchfbeinfppepibkb`.
5. Open the popup and confirm it renders the daemon status card.
6. Right-click a large file link and choose "Download with Ferric".
7. Start a normal Chrome download for a large `.zip`, `.iso`, `.mp4`, or `.pdf` file and confirm Chrome cancels it and Ferric receives the request.

Notes:
- The extension sends Rust-compatible native messages shaped as `{ "DownloadReq": { ... } }`.
- This machine's Google Chrome build rejects command-line `--load-extension` automation, so the final click/download checks must be done through Chrome's dev-mode UI.
