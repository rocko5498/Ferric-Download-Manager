export type DomainCaptureMode = "default" | "all" | "none" | "ask" | "filter"

export type CapturePolicy = {
  enabled: boolean
  minCaptureBytes: number
  extensions: string[]
  mimePrefixes: string[]
  excludeUrlPatterns: string[]
  panelExcludeHostPatterns: string[]
  captureManifests: boolean
  /** Task 117 — per-host override (hostname → mode) */
  domainPolicies: Record<string, DomainCaptureMode>
}

export const CAPTURE_POLICY_STORAGE_KEY = "ferricCapturePolicy"

// ── Daemon settings ↔ CapturePolicy conversion ────────────────────────────────

export function captureSettingsToPolicy(settings: Record<string, string>): CapturePolicy {
  return normalizeCapturePolicy({
    enabled: settings["capture_enabled"] !== "0",
    minCaptureBytes: Number(settings["min_capture_size"]) || DEFAULT_CAPTURE_POLICY.minCaptureBytes,
    extensions: parseJsonStringArray(settings["capture_extensions"]) ?? DEFAULT_CAPTURE_POLICY.extensions,
    mimePrefixes: parseJsonStringArray(settings["capture_mime_prefixes"]) ?? DEFAULT_CAPTURE_POLICY.mimePrefixes,
    excludeUrlPatterns: parseJsonStringArray(settings["capture_exclude_urls"]) ?? DEFAULT_CAPTURE_POLICY.excludeUrlPatterns,
    panelExcludeHostPatterns:
      parseJsonStringArray(settings["capture_panel_exclude_hosts"]) ??
      DEFAULT_CAPTURE_POLICY.panelExcludeHostPatterns,
    captureManifests: settings["capture_manifests"] !== "0",
    domainPolicies: parseDomainPolicies(settings["capture_domain_policies"]),
  })
}

export function policyToCaptureSettings(policy: CapturePolicy): Record<string, string> {
  return {
    capture_enabled: policy.enabled ? "1" : "0",
    min_capture_size: String(policy.minCaptureBytes),
    capture_extensions: JSON.stringify(policy.extensions),
    capture_mime_prefixes: JSON.stringify(policy.mimePrefixes),
    capture_exclude_urls: JSON.stringify(policy.excludeUrlPatterns),
    capture_panel_exclude_hosts: JSON.stringify(policy.panelExcludeHostPatterns),
    capture_manifests: policy.captureManifests ? "1" : "0",
    capture_domain_policies: JSON.stringify(policy.domainPolicies ?? {}),
  }
}

function parseDomainPolicies(raw: string | undefined): Record<string, DomainCaptureMode> {
  if (!raw) return {}
  try {
    const parsed: unknown = JSON.parse(raw)
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as Record<string, DomainCaptureMode>
    }
  } catch {
    /* ignore */
  }
  return {}
}

function parseJsonStringArray(value: string | undefined): string[] | null {
  if (!value) return null
  try {
    const parsed: unknown = JSON.parse(value)
    if (Array.isArray(parsed) && parsed.every((v) => typeof v === "string")) {
      return parsed as string[]
    }
  } catch {
    // ignore parse errors
  }
  return null
}

// Known non-download URLs seeded from IDM 6.43's own exclusion list (read from its native-messaging
// settings reply). Ads, sounds, updaters, toolbar configs.
const IDM_SEED_EXCLUDE_URLS: string[] = [
  "https://www.bing.com/api/shopping/v1/ux/images/autoApply/*.mp4",
  "http://www.slimjetbrowser.com/release/*_console.exe",
  "https://redirector.gvt1.com/edgedl/widevine-cdm/*.zip",
  "https://*.gvt1.com/edgedl/widevine-cdm/*win*.zip?*",
  "https://swx.cdn.skype.com/assets/*/audio/*.m4a",
  "http://*.ask.com/*toolbar/*config*.zip",
  "http://*.vkontakte.ru/*.zip",
  "http://toolbar.live.com/static/sm/*.bin",
  "https://sso-f.orange.fr/omoi_erb/facture/v1.0/pdf?*",
  "https://web.whatsapp.com/res/*.mp3",
  "http://*.akamaihd.net/*.mp3",
  "http://*.cloudfront.net/game/*/res/*.bin",
  "http://x.myspacecdn.com/modules/webim/static/audio/*.mp3",
  "http://img.tfd.com/hm/mp3/*.mp3",
  "http://msedge.b.tlu.dl.delivery.mp.microsoft.com/filestreamingservice/files/*",
  "https://img6.wsimg.com/serpimg/img/solutionsimg/*.mp4",
  "https://*.nic.in/*/*/*.php",
  "https://*.akamaihd.net/*.gz",
  "http://img.imgsmail.ru/*/message.bin",
  "https://www.youtube.com/s/search/audio/*.mp3",
  "http://*.zgncdn.com/*.z",
  "http://static.playfish.com/game/*.bin",
  "http://statics.verycd.com/javascripts/*.js.r*",
  "http://www.6rb.com/*.ram",
  "http://cdn.engine.*/pu.pdf",
  "http://quickaccess-dl.maxthon.com/quickaccess_*.dat*",
  "http://ciscobinary.openh264.org/*.zip",
  "https://fpdownload.macromedia.com/get/*.z",
  "http://www.cbox.ws/box/click*.wav",
  "http://6rbmasr.egyptany.net/*.ram",
  "http://widget*.meebo.com/*.mp3",
  "http://*.skyrock.com/*.img",
  "http://*.download.windowsupdate.com/*",
  "http://*.cox.com/*.gz",
  "http://ad.yieldmanager.com/st?*",
  "http://www.6rbtop.com/library/resources/*.ram",
  "http://ad.*.net/st?ad_type=*",
  "http://*.meebo.com/*skin/*/sound/*.mp3",
  "http://*.teletalk.com.bd/admitcard/card.php",
  "https://swx.cdn.skype.com/assets/*/audio/*.ogg",
  "http://ad.*.com/st?ad_type=*",
  "http://static.ak.fbcdn.net/*.ogg",
  "http://ak.imgfarm.com/images/download/spokesperson/html5/audio/*.mp3",
  "http://o.aolcdn.com/cdn.webmail.aol.com/*/aol/*/sounds/*.mp3",
  "http://counters.gigya.com/wildfire/counters/*=.tif",
  "http://images.apple.com/*/*.mov",
  "https://pc.xuexi.cn/points/study-login/asset/*.mp3",
  "https://i3j3u3u9.ssl.hwcdn.net/common/sounds/interval.mp3",
  "https://www.syssel.net/hoefs/software_count.php",
  "http://cdmdownload.adobe.com/firefox/*primetime*.zip",
  "http://get.geo.opera.com/res/servicefiles/*.zip",
  "http://*.appspot.com/*/audio/*.mp3",
  "http://*.cloudfront.net/game/*/res/*.zip",
  "http://*.farmville.com/*/sounds/*.mp3",
  "http://aa.voice2page.com/*",
  "http://imgfarm.com/images/*.bin",
  "https://lookaside.fbsbx.com/file/*.pdf?token=*",
  "http://api.browser.ya.ru/configs/all_zip?*",
  "https://ak.imgfarm.com/images/download/spokesperson/html5/audio/*.mp3",
  "https://cdmdownload.adobe.com/firefox/*primetime*.zip",
  "http://get*.opera.com/pub/opera/*/autoupdate/*.exe",
  "http://get*.opera.com/pub/opera/*/autoupdate/*.msi",
  "http://*.vk.com/*.zip",
  "http://www.sonyericsson.com/origin/images/content/*.exe",
  "https://*.nic.in/*/*/*.aspx",
  "http://assets.*.zynga.com/*.zip",
  "http://*.zynga.com/*.mp3",
  "http://www.nseries.com/nseries/*.mp3",
  "http://*.skyrock.net/*.img",
  "https://cdn.thegameawards.com/frontend/video/tga*.mp4",
  "http://village*.amazonaws.com/static/sound/*.mp3",
  "http://*.farmville.com/*.gz",
  "http://static.ak.fbcdn.net/*.mp3",
  "http://ccbar.ask.com/images/*.bin",
  "http://mail.yimg.com/*/i/us/pim/*.mp3",
  "http://*.akamaihd.net/*.zip",
  "https://*.gstatic.com/*/sounds/*.mp3",
  "http://*.zgncdn.com/*.mp3",
  "http://*.wikifortio.com/node-fs/upload/*",
  "http://movies.apple.com/*/*.mov",
  "https://*.akamaihd.net/*.mp3",
  "http://*.appspot.com/*/Sounds/*",
  "http://*.gstatic.com/*/sounds/*.mp3",
  "https://compiles.overleaf.com/project/*/output/output.pdf?*",
  "http://*.zgncdn.com/*.gz",
  "http://dar.youknowbest.com/Resources/*.img",
  "*://siteseal.thawte.com/*",
  "*://ecom.cimetz.com/*",
  "*://*.voice2page.com/*"
]

export const DEFAULT_CAPTURE_POLICY: CapturePolicy = {
  enabled: true,
  minCaptureBytes: 1_048_576,
  // IDM 6.43's own "Automatically start downloading the following file types" (read live from its
  // Options > File types). A trailing * is a prefix match (R0* = r00, r01, ...).
  extensions: [
    "3gp", "7z", "aac", "ace", "aif", "apk", "arj", "asf", "avi", "bin", "bz2", "exe", "gz", "gzip",
    "img", "iso", "lzh", "m4a", "m4v", "mkv", "mov", "mp3", "mp4", "mpa", "mpe", "mpeg", "mpg", "msi",
    "msu", "ogg", "ogv", "pdf", "plj", "pps", "ppt", "qt", "r0*", "r1*", "ra", "rar", "rm", "rmvb",
    "sea", "sit", "sitx", "tar", "tif", "tiff", "wav", "wma", "wmv", "z", "zip"
  ],

  // IDM takes over only the types on its list; a bare application/* mime (e.g. a .dat) stays with the browser.
  mimePrefixes: ["video/", "audio/"],
  excludeUrlPatterns: [
    "*://*.gstatic.com/*/sounds/*",
    "*://*.gvt1.com/edgedl/widevine-cdm/*",
    "*://*.download.windowsupdate.com/*",
    "*://download.windowsupdate.com/*",
    "*://*.update.microsoft.com/*",
    "*://*.facebook.com/*/sounds/*",
    "*://*.fbcdn.net/*/sounds/*",
    "*://*.whatsapp.com/*/res/*.mp3",
    "*://*.cloudfront.net/game/*/res/*",
    "*://*.akamaihd.net/*.mp3",
    ...IDM_SEED_EXCLUDE_URLS
  ],
  panelExcludeHostPatterns: ["*.gstatic.com", "media.tenor.co", "media*.giphy.com"],
  captureManifests: true,
  domainPolicies: {}
}

// ── Hardcoded blocklists (security filters, not user-configurable) ────────────

/** Auth/SSO domain prefixes — any hostname starting with these is blocked. */
const AUTH_DOMAIN_PREFIXES = [
  "accounts.", "login.", "oauth.", "sso.", "auth.",
  "signin.", "signup.", "id.", "idp."
]

/** Path segments that indicate auth/navigation, not real downloads. */
const NAV_PATH_SEGMENTS = [
  "/redirect", "/auth", "/login", "/logout", "/callback",
  "/signin", "/signup", "/oauth", "/sso", "/saml",
  "/authorize", "/token", "/.well-known/openid-configuration"
]

/** Query parameter keys that indicate auth flows. */
const AUTH_QUERY_KEYS = [
  "code", "token", "state", "session_id",
  "id_token", "access_token", "nonce", "redirect_uri"
]

/**
 * IDM's "never capture" types (capture settings field 2, read from its native-messaging reply): page
 * furniture and images are left to the browser even when the server sends an odd type for them.
 */
const NEVER_CAPTURE_EXTENSIONS = [
  "css", "js", "json", "crl", "ocsp", "apng", "gif", "ico", "jpeg", "jpg", "png", "svg", "webp"
]

// IDM's internal-path glob (field 21): a path with a "-internal-" folder, i.e. a site's own plumbing.
export function isInternalPath(url: string): boolean {
  try {
    return wildcardMatch("/*-internal-*/**", new URL(url).pathname)
  } catch {
    return false
  }
}

export function isNeverCaptureUrl(url: string, filename?: string | null): boolean {
  const exts = [extensionFromUrl(url)]
  if (filename) exts.push(extensionFromUrl(`file:///${filename.replace(/\\/g, "/")}`))
  return exts.some((e) => e !== null && NEVER_CAPTURE_EXTENSIONS.includes(e)) || isInternalPath(url)
}

/** MIME types that should never be captured, even if they match a prefix. */
const BLOCKED_MIME_TYPES = [
  "text/html", "text/javascript", "text/css",
  "application/json", "application/xml",
  "application/x-www-form-urlencoded",
  "application/xhtml+xml"
]

/** MIME types that count as "real media" — extensionless URLs with these pass. */
const MEDIA_MIME_TYPES = [
  "video/", "audio/",
  // application/octet-stream is deliberately absent: it is what any binary (a .dat, a game file) is
  // served as, and IDM leaves types that are not on its list to the browser.
  "application/pdf",
  "application/zip",
  "application/x-rar-compressed",
  "application/x-7z-compressed",
  "application/x-gzip",
  "application/x-tar",
  "application/x-bzip2",
  "application/vnd.apple.mpegurl",
  "application/x-mpegurl",
  "application/dash+xml"
]

export async function loadCapturePolicy(): Promise<CapturePolicy> {
  const stored = await chromeStorageGet<CapturePolicy>(CAPTURE_POLICY_STORAGE_KEY)
  return normalizeCapturePolicy(stored)
}

export async function saveCapturePolicy(policy: CapturePolicy): Promise<void> {
  await chromeStorageSet(CAPTURE_POLICY_STORAGE_KEY, normalizeCapturePolicy(policy))
}

export function normalizeCapturePolicy(policy?: Partial<CapturePolicy> | null): CapturePolicy {
  return {
    enabled: policy?.enabled ?? DEFAULT_CAPTURE_POLICY.enabled,
    minCaptureBytes: validNumber(policy?.minCaptureBytes, DEFAULT_CAPTURE_POLICY.minCaptureBytes),
    extensions: normalizeExtensions(policy?.extensions ?? DEFAULT_CAPTURE_POLICY.extensions),
    mimePrefixes: normalizeStrings(policy?.mimePrefixes ?? DEFAULT_CAPTURE_POLICY.mimePrefixes),
    excludeUrlPatterns: normalizeStrings(policy?.excludeUrlPatterns ?? DEFAULT_CAPTURE_POLICY.excludeUrlPatterns),
    panelExcludeHostPatterns: normalizeStrings(
      policy?.panelExcludeHostPatterns ?? DEFAULT_CAPTURE_POLICY.panelExcludeHostPatterns
    ),
    captureManifests: policy?.captureManifests ?? DEFAULT_CAPTURE_POLICY.captureManifests,
    domainPolicies: policy?.domainPolicies ?? DEFAULT_CAPTURE_POLICY.domainPolicies ?? {}
  }
}

export function domainModeForUrl(url: string, policy: CapturePolicy): DomainCaptureMode {
  try {
    const host = new URL(url).hostname.toLowerCase()
    const map = policy.domainPolicies ?? {}
    if (map[host]) return map[host]
    // suffix match *.example.com keys stored without leading dot
    for (const [k, mode] of Object.entries(map)) {
      if (host === k || host.endsWith(`.${k}`)) return mode
    }
  } catch {
    /* ignore */
  }
  return "default"
}

/** Is `ext` on the capture list? Entries ending in * match by prefix (IDM's R0*, R1*). */
export function hasCaptureExtension(policy: Pick<CapturePolicy, "extensions">, ext: string | null | undefined): boolean {
  if (!ext) return false
  return policy.extensions.some((e) => (e.endsWith("*") ? ext.startsWith(e.slice(0, -1)) : e === ext))
}

export function shouldCaptureUrl(url: string, policy: CapturePolicy): boolean {
  if (!policy.enabled || !isHttpUrl(url) || isExcludedUrl(url, policy) || isAuthOrNavUrl(url)) {
    return false
  }
  const mode = domainModeForUrl(url, policy)
  if (mode === "none") return false
  if (mode === "all") return true
  if (isNeverCaptureUrl(url)) return false
  const ext = extensionFromUrl(url)
  return hasCaptureExtension(policy, ext)
}

export function shouldCaptureDownloadItem(
  item: Pick<chrome.downloads.DownloadItem, "url" | "finalUrl" | "filename" | "fileSize" | "mime">,
  policy: CapturePolicy
): boolean {
  const url = item.finalUrl ?? item.url
  if (!policy.enabled || !isHttpUrl(url) || isExcludedUrl(url, policy)) {
    return false
  }
  if (isNeverCaptureUrl(url, item.filename)) return false
  const urlExt = extensionFromUrl(url)
  const filenameExt = item.filename ? extensionFromUrl(`file:///${item.filename.replace(/\\/g, "/")}`) : null
  const hasKnownExtension = Boolean(
    hasCaptureExtension(policy, urlExt) ||
    hasCaptureExtension(policy, filenameExt)
  )
  // Auth/SSO heuristics only apply to extensionless URLs: signed download links
  // legitimately carry `?token=` / `?code=` and IDM never blocks on them.
  if (!hasKnownExtension && isAuthOrNavUrl(url)) {
    return false
  }
  const lowerMime = item.mime?.toLowerCase() ?? ""
  // Reject explicitly blocked MIME types (text/html, application/json, etc.)
  // before the prefix match so they never slip through via "application/".
  if (isBlockedMime(lowerMime)) {
    return false
  }
  if (hasKnownExtension) {
    return true
  }
  // The browser already turned this navigation into a download, so there is no
  // size floor for explicit downloads (IDM only applies minimum sizes to its
  // media panel); the floor only guards extensionless MIME-only captures.
  if (item.fileSize > 0 && item.fileSize < policy.minCaptureBytes) {
    return false
  }
  if (isMediaMime(lowerMime)) {
    return true
  }
  // Attachment-style downloads with a vendor MIME (e.g. application/x-foo).
  return policy.mimePrefixes.some((prefix) => lowerMime.startsWith(prefix.toLowerCase()))
}

export function shouldCaptureManifest(url: string, mime: string | undefined, policy: CapturePolicy): boolean {
  if (!policy.enabled || !policy.captureManifests || !isHttpUrl(url) || isExcludedUrl(url, policy)) {
    return false
  }
  const lowerUrl = url.toLowerCase().split("?")[0]
  const lowerMime = mime?.toLowerCase()
  return (
    lowerUrl.endsWith(".m3u8") ||
    lowerUrl.endsWith(".mpd") ||
    lowerMime === "application/vnd.apple.mpegurl" ||
    lowerMime === "application/x-mpegurl" ||
    lowerMime === "application/dash+xml"
  )
}

export function isExcludedUrl(url: string, policy: CapturePolicy): boolean {
  return policy.excludeUrlPatterns.some((pattern) => wildcardMatch(pattern, url))
}

export function extensionFromUrl(url: string): string | null {
  let pathname = ""
  try {
    pathname = new URL(url).pathname
  } catch {
    const [withoutQuery] = url.split("?")
    pathname = withoutQuery
  }
  const name = pathname.split("/").pop() ?? ""
  const parts = name.toLowerCase().split(".").filter(Boolean)
  if (parts.length < 2) {
    return null
  }
  const tail2 = parts.slice(-2).join(".")
  if (DEFAULT_CAPTURE_POLICY.extensions.includes(tail2)) {
    return tail2
  }
  return parts[parts.length - 1] ?? null
}

export function wildcardMatch(pattern: string, value: string): boolean {
  const escaped = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".")
  return new RegExp(`^${escaped}$`, "i").test(value)
}

function normalizeExtensions(values: string[]): string[] {
  return [...new Set(values.map((value) => value.trim().toLowerCase().replace(/^\./, "")).filter(Boolean))].sort()
}

function normalizeStrings(values: string[]): string[] {
  return [...new Set(values.map((value) => value.trim()).filter(Boolean))]
}

function validNumber(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? Math.round(value) : fallback
}

function isHttpUrl(url: string): boolean {
  return url.startsWith("http://") || url.startsWith("https://")
}

/**
 * Returns true if the URL points to an auth/SSO/navigation endpoint that
 * should never be captured as a download.
 */
function isAuthOrNavUrl(url: string): boolean {
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    return false
  }
  const hostname = parsed.hostname.toLowerCase()
  if (AUTH_DOMAIN_PREFIXES.some((prefix) => hostname.startsWith(prefix))) {
    return true
  }
  const pathname = parsed.pathname.toLowerCase()
  if (NAV_PATH_SEGMENTS.some((segment) => pathname.startsWith(segment) || pathname.includes(segment + "/"))) {
    return true
  }
  const params = parsed.searchParams
  if (AUTH_QUERY_KEYS.some((key) => params.has(key))) {
    return true
  }
  return false
}

/** Returns true if the MIME type is explicitly blocked (text/html, etc.). */
function isBlockedMime(mime: string): boolean {
  if (!mime) return false
  return BLOCKED_MIME_TYPES.some((blocked) => mime === blocked || mime.startsWith(blocked + ";"))
}

/**
 * Returns true if the MIME type is a recognized media/binary type that
 * justifies capturing even when the URL has no file extension.
 */
function isMediaMime(mime: string): boolean {
  if (!mime) return false
  return MEDIA_MIME_TYPES.some((media) =>
    media.endsWith("/") ? mime.startsWith(media) : (mime === media || mime.startsWith(media + ";"))
  )
}

async function chromeStorageGet<T>(key: string): Promise<T | undefined> {
  const items = await chrome.storage.local.get(key)
  return items[key] as T | undefined
}

async function chromeStorageSet<T>(key: string, value: T): Promise<void> {
  await chrome.storage.local.set({ [key]: value })
}
