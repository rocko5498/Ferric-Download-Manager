# Store listing text

Copy these into the Edge Add-ons, Firefox Add-ons and Chrome Web Store forms.

## Name
Ferric Download Manager

## Summary (up to 132 characters)
Send browser downloads to Ferric, a fast open-source download manager with adaptive multi-connection transfers.

## Description
Ferric Download Manager takes over downloads from your browser and runs them in the Ferric desktop app: files are split across parallel connections that adapt to the server, with queues, scheduling, pause and resume, video and audio downloads, and a site grabber.

What this extension does:
- Hands browser downloads to Ferric the moment they start, and shows Ferric's own "Download File Info" window.
- Adds "Download with Ferric" to the right-click menu and "Download all links" for a page.
- Offers a "Download with Ferric" button over videos and audio on a page.
- Alt+click a link to let the browser keep it; hold Insert to force Ferric to take it.

Requires the Ferric desktop app for Windows (free and open source): https://github.com/rocko5498/Ferric-Download-Manager

The extension sends nothing to any server. It talks only to the Ferric program on your own computer.

## Category
Productivity (Edge, Chrome) / Download Management (Firefox)

## Single purpose (Chrome asks for this)
Send downloads started in the browser to the Ferric download manager installed on the same computer.

## Privacy policy
https://github.com/rocko5498/Ferric-Download-Manager/blob/main/docs/privacy-policy.md

Data use answers: the extension does not collect, sell or transfer user data; the addresses and cookies of downloads you start go only to the Ferric program on your own computer, so it can fetch the file with your login.

## Permission justifications
| Permission | Reason |
|---|---|
| `downloads` | See browser downloads so Ferric can take them over |
| `downloads.ui` | Hide the browser's own download bubble while Ferric handles the download (Chrome and Edge only) |
| `nativeMessaging` | Talk to the Ferric program on the same computer |
| `cookies` | Pass the login cookies of the download's address to Ferric |
| `webRequest` | Notice video and audio streams on a page so Ferric can offer to download them |
| `contextMenus` | Add "Download with Ferric" to the right-click menu |
| `storage` | Remember the user's capture settings |
| `scripting` | Collect links on a page for "Download all links" and detect blob downloads |
| `alarms` | Keep the connection to the local Ferric program alive |
| Host access, all sites | Let the above work on any site the user downloads from |

## Images
| File | Size | Used for |
|---|---|---|
| `icon-128.png` | 128x128 | Store icon |
| `logo-300.png` | 300x300 | Edge Add-ons logo |
| `screenshot-1.png`, `-2.png`, `-3.png` | 1280x800 | Screenshots |
| `promo-small.png` | 440x280 | Small promotional tile |
