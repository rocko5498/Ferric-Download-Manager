// One video on a streaming page is several playlist requests: a master playlist, one playlist per
// quality, often separate audio and subtitle playlists. Listing each as its own entry is what made
// the panel show "HLS stream" three times. This reads a playlist's text to tell them apart, and groups
// them under their master the way IDM does. Pure functions, no browser APIs.

export type ManifestInfo = {
  /** master: lists the qualities; media: lists the segments of one quality; unknown: unreadable. */
  role: "master" | "media" | "unknown"
  /** Video heights offered, tallest first (empty when the playlist does not say). */
  qualities: number[]
  /** Playlists this one points to (variants, audio, subtitles), absolute URLs. */
  children: string[]
}

const UNKNOWN: ManifestInfo = { role: "unknown", qualities: [], children: [] }

function absolute(uri: string, base: string): string {
  try {
    const u = new URL(uri, base)
    u.hash = ""
    return u.href
  } catch {
    return uri
  }
}

function sortedUnique(nums: number[]): number[] {
  return Array.from(new Set(nums)).filter((n) => n > 0).sort((a, b) => b - a)
}

export function parseManifest(text: string, baseUrl: string): ManifestInfo {
  const head = text.slice(0, 2048).trimStart()
  if (head.startsWith("#EXTM3U")) return parseHls(text, baseUrl)
  if (/<MPD[\s>]/.test(text.slice(0, 4096))) return parseDash(text)
  return UNKNOWN
}

function parseHls(text: string, baseUrl: string): ManifestInfo {
  const lines = text.split(/\r?\n/).map((l) => l.trim())
  const heights: number[] = []
  const children: string[] = []
  let master = false
  let media = false
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    if (line.startsWith("#EXT-X-STREAM-INF")) {
      master = true
      const res = /RESOLUTION=\d+x(\d+)/i.exec(line)
      if (res) heights.push(Number(res[1]))
      const uri = lines.slice(i + 1).find((l) => l !== "" && !l.startsWith("#"))
      if (uri) children.push(absolute(uri, baseUrl))
    } else if (line.startsWith("#EXT-X-MEDIA:") || line.startsWith("#EXT-X-I-FRAME-STREAM-INF")) {
      // Audio / subtitle / trick-play renditions only exist inside a master.
      const uri = /URI="([^"]+)"/.exec(line)
      if (uri) children.push(absolute(uri[1], baseUrl))
    } else if (line.startsWith("#EXTINF")) {
      media = true
    }
  }
  if (master) return { role: "master", qualities: sortedUnique(heights), children }
  if (media) return { role: "media", qualities: [], children: [] }
  return UNKNOWN
}

function parseDash(text: string): ManifestInfo {
  const heights = Array.from(text.matchAll(/<Representation\b[^>]*\bheight="(\d+)"/g)).map((m) => Number(m[1]))
  return { role: "master", qualities: sortedUnique(heights), children: [] }
}

type Groupable = { url: string; kind: string; info?: ManifestInfo }

const isManifest = (h: Groupable) => h.kind === "hls" || h.kind === "dash"

/**
 * One entry per video. `hits` is newest first (the order the tracker keeps them in):
 * playlists a master points to are hidden, and when playlists could not be read the first one
 * requested stands for the stream, because players ask for the master before anything else.
 */
export function groupManifests<T extends Groupable>(hits: T[]): T[] {
  const children = new Set<string>()
  for (const h of hits) if (h.info?.role === "master") for (const c of h.info.children) children.add(absolute(c, c))
  const kept = hits.filter((h) => !children.has(absolute(h.url, h.url)))
  const unreadable = kept.filter((h) => isManifest(h) && (!h.info || h.info.role === "unknown"))
  const stand = unreadable.at(-1)
  return kept.filter((h) => !unreadable.includes(h) || h === stand)
}
