/**
 * Task 110 — in-tab streaming media history (session only).
 * User chooses what to grab; nothing auto-downloads from this list.
 */

export type MediaHistoryEntry = {
  id: string
  url: string
  pageUrl?: string
  mimeType?: string
  kind: string
  tabId?: number
  observedAt: number
}

const STORAGE_KEY = "ferricMediaHistory"
const MAX_ENTRIES = 80

function store(): chrome.storage.StorageArea {
  return chrome.storage.session ?? chrome.storage.local
}

export async function pushMediaHistory(
  entry: Omit<MediaHistoryEntry, "id" | "observedAt">
): Promise<void> {
  const full: MediaHistoryEntry = {
    ...entry,
    id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    observedAt: Date.now()
  }
  const list = await loadMediaHistory()
  const filtered = list.filter((e) => e.url !== full.url)
  filtered.unshift(full)
  await new Promise<void>((resolve) => {
    store().set({ [STORAGE_KEY]: filtered.slice(0, MAX_ENTRIES) }, () => resolve())
  })
}

export async function loadMediaHistory(): Promise<MediaHistoryEntry[]> {
  return new Promise((resolve) => {
    store().get(STORAGE_KEY, (items) => {
      const v = items[STORAGE_KEY]
      resolve(Array.isArray(v) ? (v as MediaHistoryEntry[]) : [])
    })
  })
}

export async function clearMediaHistory(): Promise<void> {
  return new Promise((resolve) => {
    store().remove(STORAGE_KEY, () => resolve())
  })
}
