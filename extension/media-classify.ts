import { type CapturePolicy, isExcludedUrl, wildcardMatch } from "./capture-policy"
import type { ManifestInfo } from "./manifest-info"
import { MIME_TYPES } from "./mime-types"

// How IDM's extension decides a network response is media worth offering in the panel
//: status, content type / file type, manifests,
// ignore lists and per-type minimum sizes. Pure so it can be tested without a browser.

export type MediaHit = {
  url: string
  /** hls/dash: manifest listing every quality; direct: a plain media file. */
  kind: "direct" | "hls" | "dash"
  type: "video" | "audio"
  ext: string
  mime?: string
  /** Total size in bytes when the server told us. */
  size?: number
  /** What the playlist says it is (master / one quality / unreadable) and which playlists it points to. */
  info?: ManifestInfo
}

export type ResponseInfo = {
  url: string
  status: number
  /** Lower-cased header names. */
  headers: Record<string, string>
  /** URL of the page that made the request (for panel-excluded sites). */
  pageUrl?: string
}

/** File types the panel is enabled for (IDM registry `DwnlPanel`; `vtt` is off by default). */
const PANEL_TYPES = new Set([
  "flv", "mp3", "mp4", "m4v", "f4v", "m4a", "mpg", "mpeg", "avi", "wmv", "wma", "wav", "asf", "rm",
  "ogg", "ogv", "mov", "3gp", "qt", "webm", "mkv", "aac"
])
const AUDIO_TYPES = new Set(["mp3", "m4a", "wma", "wav", "ogg", "aac"])
/** IDM registry `DwnlPanel\minsize`: only these have a floor. */
const MIN_SIZE: Record<string, number> = { mp3: 51_200, ogg: 102_400 }
/** There is no floor for video types, but a few hundred bytes is a tracking pixel or an ad stub, never a video. */
const DEFAULT_MIN_SIZE = 102_400
/** Streaming pieces: the manifest (or the page extractor) is what the panel offers, not these. */
const SEGMENT = /\.(ts|m4s|cmfv|cmfa|cmft|fmp4|init)(?:$|[?#])/i
const CHUNK_PATH = /\/videoplayback(?:$|\?)/i
const MANIFEST_MIME = new Set([
  "application/vnd.apple.mpegurl", "application/x-mpegurl", "audio/mpegurl", "audio/x-mpegurl",
  "video/mpegurl", "application/dash+xml", "video/vnd.mpeg.dash.mpd"
])

export function headerExt(headers: Record<string, string>, url: string): string | undefined {
  const disp = headers["content-disposition"]
  const named = disp && /filename\*?=(?:UTF-8'')?"?([^";]+)/i.exec(disp)?.[1]
  const name = named ? decodeURIComponent(named) : pathName(url)
  const m = /\.([a-z0-9]{1,5})$/i.exec(name)
  return m?.[1].toLowerCase()
}

function pathName(url: string): string {
  try {
    return new URL(url).pathname.split("/").pop() ?? ""
  } catch {
    return ""
  }
}

function totalSize(headers: Record<string, string>): number | undefined {
  const range = /\/(\d+)\s*$/.exec(headers["content-range"] ?? "")
  if (range) return Number(range[1])
  const len = Number(headers["content-length"])
  return Number.isFinite(len) && len > 0 ? len : undefined
}

/** True for YouTube-style stream pieces: not offered themselves, but they mark the page as playing extractable media. */
export function isStreamChunk(url: string): boolean {
  return CHUNK_PATH.test(url)
}

export function classifyResponse(info: ResponseInfo, policy: CapturePolicy): MediaHit | null {
  if (info.status !== 200 && info.status !== 206) return null
  if (!/^https?:\/\//i.test(info.url)) return null
  if (isExcludedUrl(info.url, policy)) return null
  const hosts = [info.url, info.pageUrl ?? ""].map(hostOf).filter(Boolean)
  if (hosts.some((h) => policy.panelExcludeHostPatterns.some((p) => wildcardMatch(p, h)))) return null
  if (isStreamChunk(info.url) || SEGMENT.test(info.url)) return null

  const mime = (info.headers["content-type"] ?? "").split(";")[0].trim().toLowerCase()
  if (mime.startsWith("text/html")) return null
  const ext = headerExt(info.headers, info.url)

  const isHls = ext === "m3u8" || mime === "application/vnd.apple.mpegurl" || mime === "application/x-mpegurl"
  const isDash = ext === "mpd" || mime === "application/dash+xml" || mime === "video/vnd.mpeg.dash.mpd"
  if (isHls || isDash) {
    if (!policy.captureManifests) return null
    return { url: info.url, kind: isHls ? "hls" : "dash", type: "video", ext: isHls ? "m3u8" : "mpd", mime }
  }
  if (MANIFEST_MIME.has(mime)) return null

  const fromMime = MIME_TYPES[mime]?.split("|").map((t) => t.toLowerCase())
  const type = ext && PANEL_TYPES.has(ext) ? ext : fromMime?.find((t) => PANEL_TYPES.has(t))
  const mediaMime = mime.startsWith("video/") || mime.startsWith("audio/")
  if (!type && !mediaMime) return null
  const finalExt = type ?? (mime.split("/")[1] || "bin")
  const size = totalSize(info.headers)
  const floor = MIN_SIZE[finalExt] ?? DEFAULT_MIN_SIZE
  if (floor && size !== undefined && size < floor) return null
  // IDM drops tiny ASF files and ad-server responses.
  if (finalExt === "asf" && size !== undefined && size <= 102_400) return null
  if ((info.headers["server"] ?? "") === "DCLK-AdSvr") return null

  const audio = mime.startsWith("audio/") || AUDIO_TYPES.has(finalExt)
  return { url: info.url, kind: "direct", type: audio ? "audio" : "video", ext: finalExt, mime: mime || undefined, size }
}

function hostOf(url: string): string {
  try {
    return new URL(url).hostname.toLowerCase()
  } catch {
    return ""
  }
}

/** Pages whose video the Ferric extractor can fetch from the page URL (YouTube, plus the common sites). */
const PAGE_SITES: RegExp[] = [
  /^https?:\/\/(?:www\.|m\.|music\.)?youtube\.com\/(?:watch\?(?:.*&)?v=|shorts\/|embed\/|live\/)[\w-]{6,}/i,
  /^https?:\/\/youtu\.be\/[\w-]{6,}/i,
  /^https?:\/\/(?:www\.)?vimeo\.com\/(?:\w+\/)*\d{4,}/i,
  /^https?:\/\/(?:www\.)?dailymotion\.com\/video\/\w+/i,
  /^https?:\/\/(?:www\.|m\.|web\.)?facebook\.com\/(?:watch|reel|[^/]+\/videos|.*\/videos)\b/i,
  /^https?:\/\/(?:www\.)?instagram\.com\/(?:p|reel|reels|tv)\/[\w-]+/i,
  /^https?:\/\/(?:www\.)?(?:twitter|x)\.com\/[^/]+\/status\/\d+/i,
  /^https?:\/\/(?:www\.)?tiktok\.com\/@[^/]+\/video\/\d+/i,
  /^https?:\/\/(?:www\.)?twitch\.tv\/(?:videos\/\d+|[^/]+\/clip\/[\w-]+)/i,
  /^https?:\/\/clips\.twitch\.tv\/[\w-]+/i,
  /^https?:\/\/(?:www\.)?reddit\.com\/r\/[^/]+\/comments\/\w+/i,
  /^https?:\/\/(?:www\.)?soundcloud\.com\/[^/]+\/[^/]+/i
]

export function isPageExtractable(pageUrl: string): boolean {
  return PAGE_SITES.some((re) => re.test(pageUrl))
}
