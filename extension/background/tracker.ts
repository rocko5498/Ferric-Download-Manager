import type { CapturePolicy } from "../capture-policy"
import { groupManifests, parseManifest } from "../manifest-info"
import { type MediaHit, classifyResponse, isStreamChunk } from "../media-classify"

// Per-tab list of media seen on the network, the data behind the in-page panel. IDM reports
// every classified response to its manager keyed by tab+frame;
// here the list is kept in the service worker (mirrored to session storage so a sleeping worker
// does not lose it) and the panel asks for it.

export type TabMedia = { hits: MediaHit[]; chunk: boolean }

const KEY = "ferricTabMedia"
const MAX_HITS = 30
const tabs = new Map<number, TabMedia>()
let restored: Promise<void> | null = null
let saveTimer: ReturnType<typeof setTimeout> | undefined

function store(): chrome.storage.StorageArea {
  return chrome.storage.session ?? chrome.storage.local
}

function restore(): Promise<void> {
  restored ??= store()
    .get(KEY)
    .then((v) => {
      const saved = (v as Record<string, Record<string, TabMedia> | undefined>)[KEY] ?? {}
      for (const [id, media] of Object.entries(saved)) if (!tabs.has(Number(id))) tabs.set(Number(id), media)
    })
    .catch(() => undefined)
  return restored
}

function scheduleSave(): void {
  clearTimeout(saveTimer)
  saveTimer = setTimeout(() => void store().set({ [KEY]: Object.fromEntries(tabs) }), 300)
}

/** The tab's media, one entry per video: playlists that belong to a master are folded into it. */
export async function mediaForTab(tabId: number): Promise<TabMedia> {
  await restore()
  const entry = tabs.get(tabId) ?? { hits: [], chunk: false }
  return { hits: groupManifests(entry.hits), chunk: entry.chunk }
}

/** Read a playlist (with the page's cookies) to learn whether it is a master and which qualities it offers. */
async function describe(tabId: number, hit: MediaHit, onChange: (tabId: number) => void): Promise<void> {
  const ctl = new AbortController()
  const timer = setTimeout(() => ctl.abort(), 6000)
  try {
    const r = await fetch(hit.url, { credentials: "include", signal: ctl.signal })
    if (!r.ok) return
    hit.info = parseManifest((await r.text()).slice(0, 262_144), r.url || hit.url)
    scheduleSave()
    onChange(tabId)
  } catch {
    /* unreadable: grouping falls back to "the first playlist requested is the master" */
  } finally {
    clearTimeout(timer)
  }
}

export function clearTab(tabId: number): void {
  if (tabs.delete(tabId)) scheduleSave()
}

export function startTracker(getPolicy: () => Promise<CapturePolicy>, onChange: (tabId: number) => void): void {
  chrome.webRequest.onHeadersReceived.addListener(
    (d): undefined => {
      if (d.tabId < 0) return
      void (async () => {
        await restore()
        const policy = await getPolicy()
        if (!policy.enabled) return
        const headers: Record<string, string> = {}
        for (const h of d.responseHeaders ?? []) headers[h.name.toLowerCase()] = h.value ?? ""
        const entry = tabs.get(d.tabId) ?? { hits: [], chunk: false }
        let changed = false
        if (isStreamChunk(d.url)) {
          if (!entry.chunk) changed = entry.chunk = true
        } else {
          const hit = classifyResponse(
            { url: d.url, status: d.statusCode, headers, pageUrl: (d as { documentUrl?: string }).documentUrl ?? d.initiator },
            policy
          )
          if (hit && !entry.hits.some((h) => h.url === hit.url)) {
            entry.hits.unshift(hit)
            entry.hits.length = Math.min(entry.hits.length, MAX_HITS)
            changed = true
            if (hit.kind !== "direct") void describe(d.tabId, hit, onChange)
          }
        }
        if (changed) {
          tabs.set(d.tabId, entry)
          scheduleSave()
          onChange(d.tabId)
        }
      })()
    },
    { urls: ["<all_urls>"], types: ["media", "xmlhttprequest", "other", "object", "main_frame", "sub_frame"] },
    ["responseHeaders"]
  )
  // A new page (including a single-page-app route change) starts a fresh list.
  chrome.tabs.onUpdated.addListener((tabId, change) => {
    if (change.url) {
      // A directly opened media file answers before the URL update arrives: keep that one.
      const keep = (tabs.get(tabId)?.hits ?? []).filter((h) => h.url === change.url)
      tabs.set(tabId, { hits: keep, chunk: false })
      scheduleSave()
      onChange(tabId)
    }
  })
  chrome.tabs.onRemoved.addListener((tabId) => clearTab(tabId))
}
