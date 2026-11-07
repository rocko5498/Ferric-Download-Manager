# Ferric Download Manager: Privacy Policy

**Last updated**: 2026-11-07

## Summary

Ferric Download Manager does **not** collect, transmit, or sell any personal data about you. It has no
accounts, no telemetry, no analytics and no crash reporting. Everything Ferric stores stays on your computer.

## Network requests Ferric makes

Ferric only talks to servers **you ask it to**:

- the address of each download you start (and any mirrors, playlists or pages that download needs);
- sites you point the Site Grabber or the media extractors at;
- the proxy, FTP or SFTP server you configure;
- `https://httpbin.org/ip`, **only** when you press *Test proxy* in the proxy settings, to check that the proxy works.

Ferric never contacts a server of its own, and it does not check for updates by itself.

## Data stored on your computer

| Data                  | Purpose                                                | Where / how long                                              |
|-----------------------|--------------------------------------------------------|---------------------------------------------------------------|
| Download URLs         | To run, resume and list your downloads                 | Local SQLite database, until you delete the download          |
| File names and paths  | To save and find your files                            | Local SQLite database                                         |
| Download progress     | Shown in the interface                                 | Local SQLite database                                         |
| Browser cookies       | Sent along with a download that needs your login       | Memory only; dropped when that download ends                  |
| Saved site logins     | Only if you save one, so a site can be re-opened later | Local database, encrypted with your Windows account           |
| Diagnostic log        | Helps you find out why a download failed               | Local files in Ferric's data folder; addresses are redacted   |

Nothing in this table is sent anywhere by Ferric except as part of the requests you asked for above.

## The browser extension

The extension talks only to the Ferric program on the same computer, through the browser's native messaging
channel. It makes no network requests of its own and reads no browser cookie database; it asks the browser for
the cookies of the one address being downloaded.

| Permission          | Why it is needed                                                                   |
|---------------------|------------------------------------------------------------------------------------|
| `downloads`         | See browser downloads so Ferric can take them over                                 |
| `downloads.ui`      | Hide the browser's own download bubble while Ferric is handling the download       |
| `nativeMessaging`   | Talk to the local `ferric-host` program                                            |
| `cookies`           | Pass the login cookies of the download's address to Ferric                         |
| `webRequest`        | Notice video and audio streams on a page so Ferric can offer to download them      |
| `contextMenus`      | Add "Download with Ferric" to the right-click menu                                 |
| `storage`           | Remember your capture settings                                                     |
| `scripting`         | Collect links on a page for "Download all links" and detect blob downloads         |
| `alarms`            | Keep the connection to the local Ferric program alive                              |
| `<all_urls>` (host) | Let the above work on any site you download from                                   |

## Third-party services

None. Ferric does not use analytics, crash-reporting, advertising or sign-in services.

## Changes

If this policy changes, the new version is published in the project's GitHub repository and the
*Last updated* date above is revised.

## Contact

Open an issue on the project's GitHub repository (the *Issues* tab).
