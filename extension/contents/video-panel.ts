import "../firefox-compat"
import "webextension-polyfill"
import { type FormatRow, bestHit, formatLabel, hitLabel, panelTitle, pickMediaElement, visibleFraction } from "../media-panel"
import type { MediaHit } from "../media-classify"

// IDM-style download panel: "Download this video" over the
// page's main video, with a menu of the streams seen on the network or, on sites the extractor
// handles (YouTube...), every quality the page offers. Runs in every frame. Closed shadow DOM keeps
// page styles and scripts away from it.

export const config = { matches: ["<all_urls>"], all_frames: true }

type PanelState = { hits: MediaHit[]; extractable: boolean; pageUrl: string }
type FormatsAnswer = { title?: string; formats: FormatRow[]; subtitles?: { lang: string }[] }

const OFFSET_KEY = "ferricPanelOffset"
const MODE_KEY = "ferricPanelMini"
const PANEL_HELP = "This is the Ferric download panel. Use the arrow for more qualities, X to hide it on this page, and the menu to switch between the full and the mini panel."
let state: PanelState = { hits: [], extractable: false, pageUrl: location.href }
let host: HTMLElement | null = null
let shadow: ShadowRoot | null = null
let menuOpen = false
// IDM's "Mini mode": just a play arrow instead of the "Download this video" button.
let mini = false
let closedForPage = false
let formats: FormatsAnswer | "loading" | { error: string } | null = null
let formatsFor = ""
// Qualities of each HLS/DASH manifest seen on the page, fetched when its entry is clicked.
const manifestFormats = new Map<string, FormatsAnswer | "loading" | { error: string }>()
let status = ""
let drag: { dx: number; dy: number } | null = null
let offset = { x: 0, y: 0 }
let target: HTMLVideoElement | HTMLAudioElement | null = null
const debugLog: string[] = []
function dbg(msg: string): void {
  debugLog.push(`${Date.now() % 100000} ${msg}`)
  if (debugLog.length > 40) debugLog.shift()
}

try {
  chrome.storage.local.get(OFFSET_KEY).then((v) => {
    const o = (v as Record<string, { x: number; y: number } | undefined>)[OFFSET_KEY]
    if (o) offset = o
  })
} catch {
  /* storage unavailable: default position */
}
try {
  chrome.storage.local.get(MODE_KEY).then((v) => {
    mini = Boolean((v as Record<string, boolean | undefined>)[MODE_KEY])
    if (host) render()
  })
} catch {
  /* storage unavailable: full panel */
}

chrome.runtime.onMessage.addListener((message: { type?: string }) => {
  if (message?.type === "media_update") void refresh()
})
new MutationObserver(() => schedule()).observe(document.documentElement, { childList: true, subtree: true })
for (const ev of ["scroll", "resize"] as const) window.addEventListener(ev, () => position(), { passive: true, capture: true })
document.addEventListener("loadedmetadata", () => schedule(), true)
document.addEventListener("fullscreenchange", () => schedule())
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && menuOpen) toggleMenu(false)
}, true)

let timer: number | undefined
function schedule(): void {
  window.clearTimeout(timer)
  timer = window.setTimeout(() => void refresh(), 250)
}

async function refresh(): Promise<void> {
  try {
    const next = (await chrome.runtime.sendMessage({ type: "panel_state" })) as PanelState | undefined
    if (next) state = next
  } catch {
    return // extension reloaded or worker asleep: try on the next event
  }
  if (state.pageUrl !== formatsFor) {
    formats = null
    formatsFor = state.pageUrl
    if (menuOpen && state.extractable) void loadFormats()
  }
  render()
}

function mediaElements(): (HTMLVideoElement | HTMLAudioElement)[] {
  return Array.from(document.querySelectorAll<HTMLVideoElement | HTMLAudioElement>("video, audio"))
}

/**
 * Where the panel sits for a media element: the element itself when enough of it is on screen, else the
 * nearest ancestor that is (YouTube parks the <video> above the viewport before the first play, but its
 * player box is right there). `null` = nothing sensible to attach to.
 */
function anchorFor(el: Element): DOMRect | null {
  let cur: Element | null = el
  for (let i = 0; cur && i < 6; i++, cur = cur.parentElement) {
    const r = cur.getBoundingClientRect()
    if (r.width >= 160 && r.height >= 90 && visibleFraction(r, innerWidth, innerHeight) >= 0.2) return r
  }
  return null
}

function pickTarget(): HTMLVideoElement | HTMLAudioElement | null {
  const els = mediaElements()
  const items = els.map((el) => {
    const cs = getComputedStyle(el)
    const r = anchorFor(el)
    return { w: r?.width ?? 0, h: r?.height ?? 0, visible: !!r && cs.visibility !== "hidden" && cs.display !== "none", hovered: el.matches(":hover") }
  })
  const i = pickMediaElement(items)
  return i < 0 ? null : els[i]
}

function render(): void {
  target = closedForPage ? null : pickTarget()
  const offerable = state.hits.length > 0 || state.extractable
  if (!target || !offerable) {
    if (host) dbg(`remove host target=${Boolean(target)} offerable=${offerable} menuOpen=${menuOpen}`)
    host?.remove()
    host = shadow = null
    menuOpen = false
    return
  }
  if (!host) {
    host = document.createElement("div")
    host.style.cssText = "all:initial;position:fixed;z-index:2147483647;"
    shadow = host.attachShadow({ mode: "closed" })
    document.documentElement.appendChild(host)
  }
  build()
  position()
  prefetchFormats()
}

// Fetch the qualities before the menu is opened (IDM's list is instant), once the panel has been
// up for a moment, so pages that are skimmed past do not each cost an extraction.
let prefetchTimer: number | undefined
function prefetchFormats(): void {
  if (!state.extractable || formats !== null || prefetchTimer !== undefined) return
  const forUrl = state.pageUrl
  prefetchTimer = window.setTimeout(() => {
    prefetchTimer = undefined
    if (state.pageUrl === forUrl && formats === null && host) void loadFormats()
  }, 1200)
}

function position(): void {
  if (!host || !target) return
  const r = anchorFor(target)
  if (!r) {
    host.style.display = "none"
    return
  }
  host.style.display = ""
  // Placement: flush with the player's right edge, sitting just above its top edge;
  // with no room above, tucked into the top-right corner instead.
  const box = shadow?.querySelector(".bar")?.getBoundingClientRect()
  const w = box?.width || 170
  const h = box?.height || 28
  const above = r.top - h
  host.style.left = `${Math.max(0, Math.min(innerWidth - w, r.right - w + offset.x))}px`
  host.style.top = `${Math.max(0, (above >= 0 ? above : Math.max(r.top, 0) + 4) + offset.y)}px`
}

function build(): void {
  if (!shadow || !target) return
  const type = target.localName === "audio" ? "audio" : "video"
  const style = document.createElement("style")
  style.textContent = `
    .bar{display:flex;font:12px system-ui,sans-serif;border-radius:4px;box-shadow:0 2px 8px #0007;overflow:hidden;color:#fff;user-select:none}
    .grip{width:10px;background:#7c3a05;cursor:move}
    button{font:inherit;cursor:pointer;border:0;padding:6px 10px;background:#b45309;color:#fff}
    button:hover,button:focus-visible{background:#d06310}
    .caret{padding:6px 7px;border-left:1px solid #0003}
    .menu{margin-top:4px;background:#fff;color:#111;border-radius:4px;box-shadow:0 2px 10px #0007;min-width:230px;max-width:340px;max-height:320px;overflow:auto;font:12px system-ui,sans-serif}
    .menu button{display:block;width:100%;text-align:left;background:#fff;color:#111;padding:6px 10px;white-space:nowrap}
    .menu button:hover,.menu button:focus-visible{background:#f3e8dc}
    .menu .note{padding:6px 10px;color:#555}
    .menu hr{border:0;border-top:1px solid #ddd;margin:2px 0}
    .status{padding:4px 10px;background:#fff;color:#111;border-radius:0 0 4px 4px}`
  const bar = el("div", "bar")
  const grip = el("div", "grip")
  grip.title = "Drag to move; double-click to reset"
  grip.addEventListener("pointerdown", (e) => {
    drag = { dx: e.clientX - offset.x, dy: e.clientY - offset.y }
    grip.setPointerCapture(e.pointerId)
  })
  grip.addEventListener("pointermove", (e) => {
    if (!drag) return
    offset = { x: e.clientX - drag.dx, y: e.clientY - drag.dy }
    position()
  })
  grip.addEventListener("pointerup", () => {
    drag = null
    void chrome.storage.local.set({ [OFFSET_KEY]: offset }).catch(() => undefined)
  })
  grip.addEventListener("dblclick", () => {
    offset = { x: 0, y: 0 }
    void chrome.storage.local.set({ [OFFSET_KEY]: offset }).catch(() => undefined)
    position()
  })
  const main = el("button", "main")
  main.textContent = mini ? "▶" : panelTitle(type)
  main.title = panelTitle(type)
  main.addEventListener("click", () => void primary())
  const caret = el("button", "caret")
  caret.textContent = "▾"
  caret.setAttribute("aria-haspopup", "menu")
  caret.setAttribute("aria-expanded", String(menuOpen))
  caret.addEventListener("click", () => toggleMenu(!menuOpen))
  const help = el("button", "caret")
  help.textContent = "?"
  help.title = "About this panel"
  help.addEventListener("click", () => say(PANEL_HELP))
  const close = el("button", "caret")
  close.textContent = "✕"
  close.title = "Hide this panel on this page"
  close.addEventListener("click", () => {
    closedForPage = true
    render()
  })
  bar.append(grip, main, caret, help, close)
  const nodes: Node[] = [style, bar]
  if (menuOpen) nodes.push(menuNode())
  if (status) {
    const s = el("div", "status")
    s.textContent = status
    nodes.push(s)
  }
  shadow.replaceChildren(...nodes)
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag)
  e.className = cls
  return e
}

function menuNode(): HTMLElement {
  const menu = el("div", "menu")
  menu.setAttribute("role", "menu")
  const add = (text: string, run: () => void) => {
    const b = el("button", "item")
    b.textContent = text
    b.setAttribute("role", "menuitem")
    b.addEventListener("click", () => {
      toggleMenu(false)
      run()
    })
    menu.appendChild(b)
  }
  const note = (text: string) => {
    const n = el("div", "note")
    n.textContent = text
    menu.appendChild(n)
  }
  if (state.extractable) {
    if (formats === "loading") note("Loading qualities…")
    else if (formats && "error" in formats) note(`Could not list qualities: ${formats.error}`)
    else if (formats) {
      for (const row of formats.formats) add(formatLabel(row), () => void sendPage(row.id))
      if (formats.subtitles?.length) note(`Subtitles: ${formats.subtitles.slice(0, 4).map((s) => s.lang).join(", ")}${formats.subtitles.length > 4 ? "…" : ""}`)
    }
  }
  for (const hit of state.hits) {
    if (hit.kind === "direct") {
      add(hitLabel(hit), () => void sendHit(hit))
      continue
    }
    const open = manifestFormats.get(hit.url)
    const b = el("button", "item")
    b.textContent = `${hitLabel(hit)} ${open ? "\u25BE" : "\u25B8"}`
    b.setAttribute("role", "menuitem")
    b.addEventListener("click", () => void expandManifest(hit))
    menu.appendChild(b)
    if (open === "loading") note("  Loading qualities\u2026")
    else if (open && "error" in open) note(`  Could not list qualities: ${open.error}`)
    else if (open) {
      if (open.formats.length === 0) note("  No separate qualities")
      for (const row of open.formats) add(`    ${formatLabel(row)}`, () => void sendPage(row.id, hit.url))
    }
  }
  if (state.hits.length > 1) {
    menu.appendChild(document.createElement("hr"))
    add(`Download all (${state.hits.length} files)`, () => state.hits.forEach((h) => void sendHit(h)))
  }
  menu.appendChild(document.createElement("hr"))
  add(mini ? "Show the full panel" : "Show the mini panel", () => {
    mini = !mini
    void chrome.storage.local.set({ [MODE_KEY]: mini }).catch(() => undefined)
    menuOpen = false
    render()
  })
  add("About this panel", () => say(PANEL_HELP))
  add("Close", () => {
    closedForPage = true
    render()
  })
  return menu
}

function toggleMenu(open: boolean): void {
  dbg(`toggleMenu ${open}`)
  menuOpen = open
  if (open && state.extractable && formats === null) void loadFormats()
  build()
}

async function loadFormats(): Promise<void> {
  const forUrl = state.pageUrl
  formats = "loading"
  build()
  const answer = (await chrome.runtime.sendMessage({ type: "panel_formats", url: forUrl })) as
    | { ok: true; data: FormatsAnswer }
    | { ok: false; error: string }
  // The page may have moved on (autoplay, single-page navigation) while the daemon was extracting.
  if (forUrl !== state.pageUrl) return
  formats = answer?.ok ? answer.data : { error: answer?.error ?? "no answer" }
  build()
}

async function expandManifest(hit: MediaHit): Promise<void> {
  if (manifestFormats.has(hit.url)) {
    manifestFormats.delete(hit.url)
    return build()
  }
  manifestFormats.set(hit.url, "loading")
  build()
  const answer = (await chrome.runtime.sendMessage({ type: "panel_formats", url: hit.url })) as
    | { ok: true; data: FormatsAnswer }
    | { ok: false; error: string }
    | undefined
  manifestFormats.set(hit.url, answer?.ok ? answer.data : { error: answer?.error ?? "no answer" })
  build()
}

async function primary(): Promise<void> {
  if (state.extractable) return void (await sendPage())
  const best = bestHit(state.hits)
  if (best) await sendHit(best)
}

async function sendPage(formatId?: string, url: string = state.pageUrl): Promise<void> {
  say("Sending to Ferric…")
  const answer = (await chrome.runtime.sendMessage({ type: "panel_download", url, format_id: formatId, referrer: location.href, title: document.title })) as
    | { ok: boolean; error?: string }
    | undefined
  say(answer?.ok ? "Sent to Ferric" : `Ferric: ${answer?.error ?? "no answer"}`)
}

async function sendHit(hit: MediaHit): Promise<void> {
  await chrome.runtime.sendMessage({ type: "download_media", url: hit.url, referrer: location.href, mime: hit.mime })
  say("Sent to Ferric")
}

function say(text: string): void {
  status = text
  build()
  window.setTimeout(() => {
    status = ""
    build()
  }, 3500)
}

// Read-only snapshot for diagnosis (visible only inside the extension's isolated world).
;(globalThis as { __ferricPanel?: () => unknown }).__ferricPanel = () => ({
  menuOpen,
  closedForPage,
  hasHost: Boolean(host),
  hasTarget: Boolean(target),
  extractable: state.extractable,
  hits: state.hits.length,
  pageUrl: state.pageUrl,
  rect: target ? JSON.stringify(target.getBoundingClientRect()) : null,
  offset,
  hostTop: host?.style.top,
  targetTag: target?.localName,
  videos: document.querySelectorAll("video").length,
  formats: formats === "loading" || formats === null ? formats : "error" in formats ? formats : (formats as FormatsAnswer).formats.length,
  log: debugLog
})

void refresh()
