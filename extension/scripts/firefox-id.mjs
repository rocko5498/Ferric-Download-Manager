// Firefox only accepts a native-messaging host for an extension with a fixed id; Plasmo (0.90) has no
// per-browser manifest keys, so the id is added to the Firefox build here. Must match
// DEFAULT_FIREFOX_EXTENSION_ID in crates/ferric-host/src/install.rs.
import { readFileSync, writeFileSync } from "node:fs"
const path = new URL("../build/firefox-mv2-prod/manifest.json", import.meta.url)
const manifest = JSON.parse(readFileSync(path, "utf8"))
manifest.browser_specific_settings = { gecko: { id: "ferric@ferric.local" } }
delete manifest.key // Chrome-only
manifest.permissions = (manifest.permissions ?? []).filter((p) => p !== "downloads.ui") // Chrome-only
writeFileSync(path, JSON.stringify(manifest))
console.log("firefox manifest id set")
