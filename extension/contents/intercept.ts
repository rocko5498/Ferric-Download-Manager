import "../firefox-compat"
import "webextension-polyfill"
import { type CapturePolicy, DEFAULT_CAPTURE_POLICY, loadCapturePolicy, shouldCaptureUrl } from "../capture-policy"

let cachedPolicy: CapturePolicy = DEFAULT_CAPTURE_POLICY

void refreshPolicy()
chrome.storage.onChanged.addListener((changes, areaName) => {
  if (areaName === "local" && changes.ferricCapturePolicy) {
    cachedPolicy = (changes.ferricCapturePolicy.newValue as CapturePolicy | undefined) ?? DEFAULT_CAPTURE_POLICY
  }
})

const heldKeys = new Set<string>()
for (const type of ["keydown", "keyup"] as const) {
  document.addEventListener(
    type,
    (event) => {
      if (event.key !== "Delete" && event.key !== "Insert") return
      if (type === "keydown") heldKeys.add(event.key)
      else heldKeys.delete(event.key)
    },
    { capture: true }
  )
}
window.addEventListener("blur", () => heldKeys.clear())

document.addEventListener(
  "mousedown",
  (event) => {
    const target = event.target
    if (!(target instanceof Element)) {
      return
    }

    const anchor = target.closest("a")
    // IDM's special keys: Alt or Delete while clicking keeps the browser's own download,
    // Insert forces capture of a link the policy would ignore.
    const skip = event.altKey || heldKeys.has("Delete")
    const force = heldKeys.has("Insert")
    if (!anchor?.href || (!skip && !force && !shouldCaptureUrl(anchor.href, cachedPolicy))) {
      return
    }

    chrome.runtime.sendMessage({
      type: "pre_click",
      href: anchor.href,
      referrer: location.href,
      skip,
      force
    })
  },
  { capture: true }
)

async function refreshPolicy(): Promise<void> {
  try {
    cachedPolicy = await loadCapturePolicy()
  } catch {
    cachedPolicy = DEFAULT_CAPTURE_POLICY
  }
}

function isHttpUrl(url: string): boolean {
  return url.startsWith("http://") || url.startsWith("https://")
}
