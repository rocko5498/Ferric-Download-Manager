# Ferric — Enterprise Deployment Guide

## Chrome / Edge — Force-install via Group Policy

### Windows GPO

1. Download the Chrome ADMX templates from Google and install to `%SystemRoot%\PolicyDefinitions\`.
2. Open **Group Policy Management Editor** → Computer Configuration → Administrative Templates → Google Chrome → Extensions.
3. Set **Configure the list of force-installed apps and extensions** to Enabled.
4. Add entry: `<extension-id>;https://clients2.google.com/service/update2/crx`

Replace `<extension-id>` with the Ferric Chrome Web Store ID once published.

Alternatively, set via registry:

```
HKLM\SOFTWARE\Policies\Google\Chrome\ExtensionInstallForcelist
  1 = "<extension-id>;https://clients2.google.com/service/update2/crx"
```

For **Microsoft Edge**, replace `Google\Chrome` with `Microsoft\Edge`.

### ExtensionSettings policy (recommended — controls pinning + permissions)

Place `extension/policy/ferric.json` content under:

```
HKLM\SOFTWARE\Policies\Google\Chrome\ExtensionSettings
  value = <JSON string from ferric.json with real extension ID>
```

### macOS MDM / Jamf

Deploy via a `.mobileconfig` profile or Jamf policy:

```xml
<key>ExtensionInstallForcelist</key>
<array>
  <string><extension-id>;https://clients2.google.com/service/update2/crx</string>
</array>
```

### Linux (JSON policy file)

Create `/etc/opt/chrome/policies/managed/ferric.json` with contents from
`extension/policy/ferric.json` (substitute real extension ID).

For Edge on Linux: `/etc/opt/edge/policies/managed/ferric.json`.

---

## Native Messaging Host — Fleet Deployment

The `ferric-host --install <extension-id>` command registers the native messaging
host for the current user. For fleet deployment:

### Windows — per-machine registration (requires admin)

```powershell
$hostPath = "C:\Program Files\Ferric\ferric-host.exe"
$manifestPath = "C:\Program Files\Ferric\com.ferric.host.chrome.json"

# Write manifest
Copy-Item "native-messaging\com.ferric.host.chrome.json" $manifestPath
(Get-Content $manifestPath) -replace '"path": ".*"', '"path": "' + $hostPath + '"' | Set-Content $manifestPath

# Register for all users (HKLM)
reg add "HKLM\SOFTWARE\Google\Chrome\NativeMessagingHosts\com.ferric.host" /ve /t REG_SZ /d $manifestPath /f
reg add "HKLM\SOFTWARE\Microsoft\Edge\NativeMessagingHosts\com.ferric.host" /ve /t REG_SZ /d $manifestPath /f
```

### Linux — per-machine

```bash
sudo cp native-messaging/com.ferric.host.chrome.json \
  /etc/opt/chrome/native-messaging-hosts/com.ferric.host.json
sudo cp native-messaging/com.ferric.host.chrome.json \
  /etc/opt/edge/native-messaging-hosts/com.ferric.host.json
# Edit "path" in each file to point to the installed ferric-host binary
```

---

## Managed Preferences (pre-configure extension settings)

Deploy `extension/policy/managed_preferences.json` as an ExtensionSettings
`managed_storage` entry or via an enterprise managed storage schema.

Fields:
| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `capturePolicyEnabled` | bool | true | Enable/disable automatic download capture |
| `captureExtensions` | string[] | see file | File extensions to auto-capture |
| `captureMinSizeBytes` | int | 1048576 (1 MB) | Minimum file size to capture |
| `daemonAutoStart` | bool | true | Start `ferricd` on system login |
