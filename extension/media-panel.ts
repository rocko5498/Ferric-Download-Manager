/** Pure logic for the in-page download panel (no DOM/chrome APIs, so it is unit-tested). Wording and
 * formats follow IDM's panel. */

import type { MediaHit } from "./media-classify"

/** One row of the daemon's `ExtractFormats` answer. */
export type FormatRow = {
  id: string
  ext: string
  height?: number | null
  fps?: number | null
  size?: number | null
  audio_only?: boolean
  merge?: boolean
  abr?: number | null
  tbr?: number | null
  vcodec?: string | null
  protocol?: string | null
}

/** `%d<decimal separator>%02d <unit>`; below 1 KB just `%d Bytes`. */
export function formatSize(bytes: number, locale?: string): string {
  if (!Number.isFinite(bytes) || bytes < 0) return ""
  if (bytes < 1024) return `${Math.round(bytes)} Bytes`
  const units = ["KB", "MB", "GB", "TB"]
  let value = bytes / 1024
  let i = 0
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024
    i++
  }
  const text = value.toLocaleString(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 })
  return `${text} ${units[i]}`
}

/** "Download this video" / "Download this audio" (IDM `Download this %s`). */
export function panelTitle(type: "video" | "audio"): string {
  return `Download this ${type}`
}

export function hitLabel(hit: MediaHit, locale?: string): string {
  if (hit.kind === "hls" || hit.kind === "dash") {
    const name = hit.kind === "hls" ? "HLS stream" : "DASH stream"
    if (hit.info?.role === "media") return `${name} (single quality)`
    const q = hit.info?.qualities ?? []
    if (q.length === 0) return name
    return `${name} (${q.slice(0, 4).map((h) => `${h}p`).join(", ")}${q.length > 4 ? ", \u2026" : ""})`
  }
  const size = hit.size ? formatSize(hit.size, locale) : ""
  return size ? `${hit.ext.toUpperCase()}  ${size}` : hit.ext.toUpperCase()
}

/**
 * What the main "Download this video" button should take: a master playlist (it lists every quality),
 * else the biggest direct file, else a single-quality playlist.
 */
export function bestHit(hits: MediaHit[]): MediaHit | undefined {
  const rank = (h: MediaHit): [number, number, number] =>
    h.kind === "direct"
      ? [1, h.size ?? 0, 0]
      : h.info?.role === "media"
        ? [0, 0, 0]
        : [2, h.info?.qualities.length ?? 0, h.info?.qualities[0] ?? 0]
  return [...hits].sort((a, b) => {
    const [ra, rb] = [rank(a), rank(b)]
    return rb[0] - ra[0] || rb[1] - ra[1] || rb[2] - ra[2]
  })[0]
}

/** "avc1.64002a" -> "H.264", so rows that differ only by codec can be told apart. */
export function codecName(vcodec?: string | null): string {
  const c = (vcodec ?? "").toLowerCase()
  if (c.startsWith("avc")) return "H.264"
  if (c.startsWith("hvc") || c.startsWith("hev")) return "H.265"
  if (c.startsWith("vp09") || c.startsWith("vp9")) return "VP9"
  if (c.startsWith("vp8")) return "VP8"
  if (c.startsWith("av01")) return "AV1"
  return ""
}

export function formatLabel(row: FormatRow, locale?: string): string {
  const quality = row.audio_only
    ? `Audio${row.abr ? ` ${Math.round(row.abr)}k` : ""}`
    : `${row.height ?? "?"}p${row.fps && row.fps > 30 ? Math.round(row.fps) : ""}`
  const container = row.ext === "m3u8" ? "HLS" : row.ext.toUpperCase()
  const codec = row.audio_only ? "" : codecName(row.vcodec)
  // Playlists have no file size; their bitrate is what tells the variants apart.
  const rate = !row.size && row.tbr ? (row.tbr >= 1000 ? `${(row.tbr / 1000).toFixed(1)} Mbps` : `${Math.round(row.tbr)} kbps`) : ""
  const size = row.size ? formatSize(row.size, locale) : ""
  const tail = [rate, size].filter(Boolean).join(" ")
  return `${quality} ${[container, codec].filter(Boolean).join(" ")}${tail ? `  ${tail}` : ""}`
}

export type Box = { left: number; top: number; right: number; bottom: number; width: number; height: number }

/** Share of a box that lies inside the viewport (IDM shows a panel only when at least 20% of the video is visible). */
export function visibleFraction(r: Box, viewportW: number, viewportH: number): number {
  const area = r.width * r.height
  if (area <= 0) return 0
  const w = Math.max(0, Math.min(r.right, viewportW) - Math.max(r.left, 0))
  const h = Math.max(0, Math.min(r.bottom, viewportH) - Math.max(r.top, 0))
  return (w * h) / area
}

/**
 * The element worth decorating (IDM `Tb`): the one under the pointer or focused if it is visible,
 * otherwise the largest visible one with a sane aspect ratio. Returns an index or -1.
 */
export type ElementInfo = { w: number; h: number; visible: boolean; hovered?: boolean }

export function pickMediaElement(items: ElementInfo[]): number {
  const usable = (s: ElementInfo) => s.visible && s.w >= 160 && s.h >= 90
  const hovered = items.findIndex((s) => s.hovered && usable(s))
  if (hovered >= 0) return hovered
  let best = -1
  let area = 0
  items.forEach((s, i) => {
    const a = s.w * s.h
    // IDM: width * 1.35 > height and width < height * 3 (rejects slivers and banners).
    if (usable(s) && s.w * 1.35 > s.h && s.w < s.h * 3 && a > area) {
      best = i
      area = a
    }
  })
  return best
}
