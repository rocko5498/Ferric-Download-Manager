import {
  type CapturePolicy,
  domainModeForUrl,
  isExcludedUrl,
  isNeverCaptureUrl,
  shouldCaptureDownloadItem
} from "./capture-policy"
import { MIME_TYPES } from "./mime-types"

export type GrabInput = {
  url: string
  filename?: string
  mime?: string
  fileSize?: number
  /** Insert held while clicking: capture even if the policy says no. */
  forced: boolean
  /** Alt/Delete held while clicking: the browser keeps the download. */
  skipped: boolean
}

export type GrabDecision = { capture: boolean; reason: string }

/**
 * IDM's takeover decision, taken once the browser knows the real file name and type
 * (downloads.onDeterminingFilename).
 */
export function decideGrab(input: GrabInput, policy: CapturePolicy): GrabDecision {
  if (!/^https?:\/\//i.test(input.url)) return { capture: false, reason: "not_http" }
  if (input.skipped) return { capture: false, reason: "skip_key" }
  if (!policy.enabled) return { capture: false, reason: "policy_disabled" }
  if (input.forced) return { capture: true, reason: "force_key" }
  const mode = domainModeForUrl(input.url, policy)
  if (mode === "none") return { capture: false, reason: "site_never" }
  if (mode === "all") return { capture: true, reason: "site_always" }
  if (isExcludedUrl(input.url, policy)) return { capture: false, reason: "excluded_url" }
  if (isNeverCaptureUrl(input.url, input.filename)) return { capture: false, reason: "never_capture_type" }
  const item = {
    url: input.url,
    finalUrl: input.url,
    filename: input.filename ?? "",
    fileSize: input.fileSize ?? 0,
    mime: input.mime ?? ""
  }
  if (shouldCaptureDownloadItem(item, policy)) return { capture: true, reason: "policy" }
  const types = MIME_TYPES[(input.mime ?? "").split(";")[0].trim().toLowerCase()]
  if (types?.split("|").some((t) => policy.extensions.includes(t.toLowerCase()))) {
    return { capture: true, reason: "mime_type" }
  }
  return { capture: false, reason: "not_in_capture_list" }
}
