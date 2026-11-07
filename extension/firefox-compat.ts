// Firefox's `chrome.*` calls take callbacks and return nothing; its `browser.*` returns promises like
// Chrome MV3's `chrome.*` does. This code uses the promise style, so on Firefox point `chrome` at
// `browser`. `getBrowserInfo` exists only in Firefox (the polyfill also defines `browser` on Chrome,
// which must stay untouched). Import this first in every entry point.
const native = (globalThis as { browser?: { runtime?: { getBrowserInfo?: unknown } } }).browser
if (typeof native?.runtime?.getBrowserInfo === "function") {
  ;(globalThis as { chrome?: unknown }).chrome = native
}
export {}
