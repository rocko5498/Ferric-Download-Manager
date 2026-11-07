# Ferric Native Messaging Host

Extension ID: `gepkkfjfgdcgelcnejpikdeanggknbjo` (the packed `Ferric.crx`, signed with the Ferric key; see `scripts/pack-extension.ps1`). Earlier unpacked builds used `bngipfinnjpcnebfchfbeinfppepibkb`, which the host still accepts.

Build the host first:

```powershell
cargo build -p ferric-host
```

Install for Chrome on Windows:

```powershell
powershell -ExecutionPolicy Bypass -File .\native-messaging\install-chrome.ps1
```

Install for Chrome on Linux:

```sh
sh ./native-messaging/install-chrome.sh
```

The scripts generate a Chrome native messaging manifest from
`com.ferric.host.chrome.json` and register it for the current user.
