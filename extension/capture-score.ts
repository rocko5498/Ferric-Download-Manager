/**
 * Task 116 — ML-free capture scorer.
 * Score ≥ threshold → capture; below → ignore (unless force).
 */

import { type CapturePolicy, extensionFromUrl, hasCaptureExtension, isExcludedUrl, shouldCaptureManifest } from "./capture-policy"

export type ScoreInput = {
  url: string
  mime?: string
  fileSize?: number
  filename?: string
}

export type ScoreResult = {
  score: number
  capture: boolean
  reasons: string[]
}

const DEFAULT_THRESHOLD = 5

export function scoreCapture(
  input: ScoreInput,
  policy: CapturePolicy,
  threshold = DEFAULT_THRESHOLD
): ScoreResult {
  const reasons: string[] = []
  let score = 0

  if (!policy.enabled) {
    return { score: 0, capture: false, reasons: ["policy_disabled"] }
  }
  if (!input.url.startsWith("http://") && !input.url.startsWith("https://")) {
    return { score: 0, capture: false, reasons: ["not_http"] }
  }
  if (isExcludedUrl(input.url, policy)) {
    return { score: 0, capture: false, reasons: ["excluded_pattern"] }
  }

  const ext = extensionFromUrl(input.url)
  if (hasCaptureExtension(policy, ext)) {
    score += 4
    reasons.push(`ext:${ext}`)
  }
  if (input.filename) {
    const fe = extensionFromUrl(`file:///${input.filename.replace(/\\/g, "/")}`)
    if (hasCaptureExtension(policy, fe)) {
      score += 3
      reasons.push(`filename_ext:${fe}`)
    }
  }

  const mime = (input.mime ?? "").toLowerCase()
  if (mime.startsWith("video/") || mime.startsWith("audio/")) {
    score += 5
    reasons.push("media_mime")
  } else if (mime === "application/pdf" || mime.includes("zip") || mime.includes("octet-stream")) {
    score += 3
    reasons.push("binary_mime")
  } else if (mime.startsWith("text/") || mime.includes("json") || mime.includes("javascript")) {
    score -= 8
    reasons.push("text_mime_penalty")
  }

  if (shouldCaptureManifest(input.url, input.mime, policy)) {
    score += 6
    reasons.push("manifest")
  }

  if (typeof input.fileSize === "number" && input.fileSize > 0) {
    if (input.fileSize >= policy.minCaptureBytes) {
      score += 2
      reasons.push("size_ok")
    } else {
      score -= 4
      reasons.push("size_too_small")
    }
  }

  // URL structure hints
  const path = input.url.toLowerCase().split("?")[0]
  if (/\.(mp4|mkv|webm|zip|pdf|iso|dmg)(\b|$)/i.test(path)) {
    score += 2
    reasons.push("path_ext")
  }
  if (/\/(cdn|download|files|media|video)\//i.test(path)) {
    score += 1
    reasons.push("path_hint")
  }

  return {
    score,
    capture: score >= threshold,
    reasons
  }
}
