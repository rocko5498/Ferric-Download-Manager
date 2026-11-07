import "../firefox-compat"
// Firefox compatibility: re-exports chrome.* as browser.* on Chromium; uses native browser.* on Firefox
import browser from "webextension-polyfill"
import {
  type CapturePolicy,
  captureSettingsToPolicy,
  loadCapturePolicy,
  policyToCaptureSettings,
  saveCapturePolicy,
  shouldCaptureManifest,
  shouldCaptureUrl
} from "../capture-policy"
import { scoreCapture } from "../capture-score"
import { decideGrab } from "../grab"
import { isPageExtractable } from "../media-classify"
import { settleAck, waitForAck } from "./ack"
import { mediaForTab, startTracker } from "./tracker"
import { loadMediaHistory, pushMediaHistory, clearMediaHistory } from "../media-history"

type DownloadRequest = {
  id: string
  url: string
  final_url?: string
  filename?: string
  referrer?: string
  cookies?: string
  headers?: Record<string, string>
  size_hint?: number
  mime_type?: string
  method: "GET" | "POST"
  post_body_b64?: string
  timestamp: number
}

type MediaKind =
  | "direct_file"
  | "hls_manifest"
  | "dash_manifest"
  | "media_segment"
  | "unsupported"
  | "protected_or_expired"

type MediaCandidate = {
  id: string
  url: string
  page_url?: string
  mime_type?: string
  kind: MediaKind
  observed_at: number
}

type HostMessage =
  | { DownloadReq: DownloadRequest }
  | { MediaCandidate: MediaCandidate }
  | "GetCapturePolicy"
  | { SetCapturePolicy: { settings: Record<string, string> } }
  | { ExtractFormats: { url: string } }
  | { PageDownload: { url: string; format_id?: string; cookies?: string; referrer?: string; title?: string } }

type DaemonMessage =
  | { type: "accepted"; id: string }
  | { type: "error"; id?: string; message: string }
  | { type: "pong"; version: string }
  | { type: "site_rules"; rules: unknown[] }
  | { type: "capture_policy"; settings: Record<string, string> }
  | { type: "ok" }
  | { type: "formats"; data: unknown }

type PopupMessage =
  | { type: "get_status" }
  | { type: "open_ferric" }
  | { type: "pre_click"; href: string; referrer: string; skip?: boolean; force?: boolean }
  | { type: "download_media"; url: string; referrer: string; mime?: string }
  | { type: "panel_state" }
  | { type: "panel_formats"; url: string }
  | { type: "panel_download"; url: string; format_id?: string; referrer?: string; title?: string }
  | { type: "sync_policy_to_daemon" }
  | { type: "get_media_history" }
  | { type: "clear_media_history" }
  | { type: "grab_media_history"; url: string; referrer?: string }

const HOST_NAME = "com.ferric.host"
const CONTEXT_MENU_ID = "ferric-download-link"
const CONTEXT_MENU_DOWNLOAD_ALL_ID = "ferric-download-all"

let port: chrome.runtime.Port | null = null
let daemonConnected = false
let lastError: string | null = null
let pendingPolicySync: Array<(policy: CapturePolicy) => void> = []

chrome.alarms.create("keepalive", { periodInMinutes: 0.4 })
chrome.alarms.onAlarm.addListener(() => {
  // The service worker may have slept through the daemon stopping: never leave the browser's own
  // download bubble hidden while Ferric is not there to take over.
  if (!daemonConnected) setBrowserDownloadUi(true)
})

// IDM hides the browser's download bubble (its extension asks for downloads.ui). Ferric shows its own
// start dialog, so the bubble would only flash for a download Ferric has just taken over. Hidden only
// while the daemon answers; Firefox has no such API.
let bubbleHidden = false
function setBrowserDownloadUi(show: boolean): void {
  if (!show && bubbleHidden) return
  bubbleHidden = !show
  try {
    void chrome.downloads.setUiOptions?.({ enabled: show })?.catch?.(() => undefined)
  } catch {
    /* not supported here */
  }
}

chrome.runtime.onInstalled.addListener(createContextMenu)
chrome.runtime.onStartup.addListener(createContextMenu)

// IDM's takeover: freeze the browser download as soon as it exists, then decide once the
// browser knows the real file name/type, keeping Chrome's filename step open until we answer.
const HOLD_TIMEOUT_MS = 12_000
// How long Ferric has to answer a capture before the browser keeps its own download.
const ACK_TIMEOUT_MS = 8_000
const heldDownloads = new Map<number, () => void>()

// Firefox has no onDeterminingFilename: there the decision runs right after the pause, with
// whatever name and type the browser has by then.
const hasFilenameStep = typeof chrome.downloads.onDeterminingFilename?.addListener === "function"

chrome.downloads.onCreated.addListener((item) => {
  const url = item.finalUrl ?? item.url
  if (!isHttpUrl(url)) return
  // Firefox cannot resume a download it paused before the transfer started, so there Ferric decides
  // first and only cancels the browser's copy once it has taken the download.
  if (hasFilenameStep) void pauseChromeDownload(item.id)
  else void decideDownload(item, () => undefined)
})

if (hasFilenameStep) {
  chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
    if (!isHttpUrl(item.finalUrl ?? item.url)) return false
    void decideDownload(item, suggest)
    return true
  })
}

chrome.contextMenus.onClicked.addListener((info, tab) => {
  void captureContextMenuClick(info, tab)
})

chrome.runtime.onMessage.addListener((message: PopupMessage, sender, sendResponse) => {
  if (sender.id !== chrome.runtime.id) return false

  if (message.type === "get_status") {
    sendResponse({
      daemonConnected,
      lastError
    })
    return false
  }

  if (message.type === "open_ferric") {
    // The daemon/UI show-window command is owned by Task 008. Keep this as a no-op handshake.
    sendResponse({ ok: true })
    return false
  }

  if (message.type === "pre_click") {
    if (isHttpUrl(message.href)) {
      // storage.session not available in Firefox MV2; fall back to local
      const sessionStore = chrome.storage.session ?? chrome.storage.local
      void sessionStore.set({
        ferricPreClick: {
          href: message.href,
          referrer: message.referrer,
          skip: message.skip === true,
          force: message.force === true,
          timestamp: Date.now()
        }
      })
    }
    sendResponse({ ok: true })
    return false
  }

  if (message.type === "panel_state") {
    const tabId = sender.tab?.id
    const pageUrl = sender.url ?? sender.tab?.url ?? ""
    if (tabId === undefined) {
      sendResponse({ hits: [], extractable: false, pageUrl })
      return false
    }
    void Promise.all([mediaForTab(tabId), loadCurrentCapturePolicy()]).then(([media, policy]) =>
      sendResponse({
        hits: media.hits,
        extractable: policy.enabled && isPageExtractable(pageUrl),
        pageUrl,
        panelExcluded: policy.panelExcludeHostPatterns
      })
    )
    return true
  }

  if (message.type === "panel_formats") {
    void hostRequest({ ExtractFormats: { url: message.url } })
      .then((reply) =>
        sendResponse(reply.type === "formats" ? { ok: true, data: reply.data } : { ok: false, error: replyError(reply) })
      )
      .catch((error: unknown) => sendResponse({ ok: false, error: String(error) }))
    return true
  }

  if (message.type === "panel_download") {
    void (async () => {
      try {
        const reply = await hostRequest({
          PageDownload: {
            url: message.url,
            format_id: message.format_id,
            cookies: await cookieString(message.url),
            referrer: message.referrer ?? message.url,
            title: message.title
          }
        })
        sendResponse(reply.type === "accepted" ? { ok: true } : { ok: false, error: replyError(reply) })
      } catch (error) {
        sendResponse({ ok: false, error: String(error) })
      }
    })()
    return true
  }

  if (message.type === "download_media") {
    void downloadMedia(message)
    sendResponse({ ok: true })
    return false
  }

  if (message.type === "sync_policy_to_daemon") {
    void (async () => {
      const policy = await loadCapturePolicy()
      const settings = policyToCaptureSettings(policy)
      postHostMessage({ SetCapturePolicy: { settings } })
      await syncPolicyFromDaemon({ timeoutMs: 1500 })
    })()
    sendResponse({ ok: true })
    return false
  }

  if (message.type === "get_media_history") {
    void loadMediaHistory().then((entries) => sendResponse({ entries }))
    return true
  }

  if (message.type === "clear_media_history") {
    void clearMediaHistory().then(() => sendResponse({ ok: true }))
    return true
  }

  if (message.type === "grab_media_history") {
    void (async () => {
      const policy = await loadCurrentCapturePolicy()
      const scored = scoreCapture({ url: message.url }, policy)
      if (!scored.capture && !shouldCaptureUrl(message.url, policy)) {
        sendResponse({ ok: false, score: scored.score, reasons: scored.reasons })
        return
      }
      await downloadMedia({ type: "download_media", url: message.url, referrer: message.referrer ?? "" })
      sendResponse({ ok: true, score: scored.score })
    })()
    return true
  }

  return false
})

/** One-shot request on its own native connection, so a slow extraction never blocks download captures. */
function hostRequest(message: HostMessage, timeoutMs = 90_000): Promise<DaemonMessage> {
  return new Promise((resolve, reject) => {
    const p = chrome.runtime.connectNative(HOST_NAME)
    const timer = globalThis.setTimeout(() => {
      p.disconnect()
      reject(new Error("Ferric did not answer in time"))
    }, timeoutMs)
    p.onMessage.addListener((reply: DaemonMessage) => {
      globalThis.clearTimeout(timer)
      p.disconnect()
      resolve(reply)
    })
    p.onDisconnect.addListener(() => {
      globalThis.clearTimeout(timer)
      reject(new Error(chrome.runtime.lastError?.message ?? "Ferric is not running"))
    })
    p.postMessage(message)
  })
}

function replyError(reply: DaemonMessage): string {
  return reply.type === "error" ? reply.message : "unexpected reply from Ferric"
}

startTracker(loadCapturePolicy, (tabId) => {
  chrome.tabs.sendMessage(tabId, { type: "media_update" }).catch(() => undefined)
})

function getPort(): chrome.runtime.Port {
  if (!port) {
    port = chrome.runtime.connectNative(HOST_NAME)
    // Not "connected" yet: only an answer from Ferric proves the host and the daemon are really there.
    lastError = null
    port.onDisconnect.addListener(() => {
      lastError = chrome.runtime.lastError?.message ?? "native host disconnected"
      trace(`disconnect ${lastError}`)
      daemonConnected = false
      setBrowserDownloadUi(true)
      port = null
      pendingPolicySync = []
    })
    port.onMessage.addListener(handleDaemonMessage)
    // Fetch capture policy from daemon on every new connection so daemon stays
    // the source of truth. The response arrives via handleDaemonMessage.
    port.postMessage("GetCapturePolicy" satisfies HostMessage)
  }
  return port
}

function handleDaemonMessage(message: DaemonMessage): void {
  daemonConnected = true
  setBrowserDownloadUi(false)
  if (message.type === "accepted") {
    settleAck(message.id, true)
    lastError = null
    return
  }
  if (message.type === "error") {
    settleAck(message.id, false)
    lastError = message.message
    return
  }
  if (message.type === "capture_policy") {
    // Daemon is the source of truth — persist its policy to local storage so
    // content scripts and the popup can read it without a native message round-trip.
    const policy = captureSettingsToPolicy(message.settings)
    void saveCapturePolicy(policy)
    const waiters = pendingPolicySync.splice(0)
    for (const resolve of waiters) {
      resolve(policy)
    }
  }
}

async function loadCurrentCapturePolicy(timeoutMs = 1500): Promise<CapturePolicy> {
  const daemonPolicy = await syncPolicyFromDaemon({ timeoutMs })
  return daemonPolicy ?? loadCapturePolicy()
}

function syncPolicyFromDaemon(options?: { timeoutMs?: number }): Promise<CapturePolicy | null> {
  const timeoutMs = options?.timeoutMs ?? 1500
  return new Promise((resolve) => {
    let finished = false
    let timer: ReturnType<typeof setTimeout>
    const waiter = (policy: CapturePolicy) => finish(policy)
    const finish = (policy: CapturePolicy | null) => {
      if (finished) return
      finished = true
      globalThis.clearTimeout(timer)
      const idx = pendingPolicySync.indexOf(waiter)
      if (idx >= 0) {
        pendingPolicySync.splice(idx, 1)
      }
      resolve(policy)
    }

    timer = globalThis.setTimeout(() => finish(null), timeoutMs)
    pendingPolicySync.push(waiter)
    try {
      getPort().postMessage("GetCapturePolicy" satisfies HostMessage)
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
      finish(null)
    }
  })
}

function postHostMessage(message: HostMessage): void {
  getPort().postMessage(message)
}

async function decideDownload(
  item: chrome.downloads.DownloadItem,
  suggest: (suggestion?: chrome.downloads.FilenameSuggestion) => void
): Promise<void> {
  const url = item.finalUrl ?? item.url
  let answered = false
  // Hand the download back to the browser (once): answer its filename step, then resume.
  const release = async () => {
    if (answered) return
    answered = true
    heldDownloads.delete(item.id)
    suggest()
    trace(`suggest ${item.id}`)
    if (hasFilenameStep) await resumeChromeDownload(item.id)
  }
  heldDownloads.set(item.id, () => void release())
  const timer = globalThis.setTimeout(() => void release(), HOLD_TIMEOUT_MS)
  try {
    const [skipped, forced, policy] = await Promise.all([
      userSkippedCapture(url, item.url),
      userForcedCapture(url, item.url),
      // The first download after the browser starts may have to wait for the host to start Ferric.
      loadCurrentCapturePolicy(daemonConnected ? 1500 : 6000)
    ])
    const decision = decideGrab(
      { url, filename: basename(item.filename), mime: item.mime, fileSize: item.fileSize, forced, skipped },
      policy
    )
    trace(`decide ${item.id} ${decision.reason} capture=${decision.capture} daemon=${daemonConnected} name=${item.filename} mime=${item.mime}`)
    // Daemon unreachable: never cancel the browser's download and lose it.
    if (!decision.capture || !daemonConnected) {
      await release()
      return
    }
    answered = true
    heldDownloads.delete(item.id)
    const requestId = crypto.randomUUID()
    const acked = waitForAck(requestId, ACK_TIMEOUT_MS)
    postDownloadRequest({
      id: requestId,
      url,
      final_url: item.finalUrl,
      filename: basename(item.filename),
      referrer: item.referrer,
      cookies: await cookieString(url),
      headers: { "user-agent": navigator.userAgent },
      size_hint: item.fileSize > 0 ? item.fileSize : undefined,
      mime_type: item.mime,
      method: "GET",
      timestamp: Date.now()
    })
    // Cancel the browser's copy only once Ferric has it; otherwise the browser carries on with it.
    if (!(await acked)) {
      trace(`no ack ${item.id}: leaving the download to the browser`)
      answered = false
      await release()
      return
    }
    await cancelChromeDownload(item.id)
    await eraseChromeDownload(item.id)
  } catch (error) {
    lastError = error instanceof Error ? error.message : String(error)
    await release()
  } finally {
    globalThis.clearTimeout(timer)
  }
}

async function captureContextMenuClick(
  info: chrome.contextMenus.OnClickData,
  tab?: chrome.tabs.Tab
): Promise<void> {
  if (info.menuItemId === CONTEXT_MENU_DOWNLOAD_ALL_ID) {
    await captureAllLinksFromTab(tab)
    return
  }

  const url = info.linkUrl ?? info.srcUrl
  if (!url || !isHttpUrl(url)) {
    return
  }

  postDownloadRequest({
    id: crypto.randomUUID(),
    url,
    referrer: tab?.url,
    cookies: await cookieString(url),
    method: "GET",
    timestamp: Date.now()
  })
}

async function captureAllLinksFromTab(tab?: chrome.tabs.Tab): Promise<void> {
  if (tab?.id == null) {
    lastError = "No active tab available for Download All"
    return
  }

  const policy = await loadCurrentCapturePolicy()
  const urls = await harvestTabLinks(tab.id)
  const uniqueUrls = [...new Set(urls)]
    .filter((url) => shouldCaptureUrl(url, policy))

  if (uniqueUrls.length === 0) {
    lastError = "No downloadable links matched the capture policy"
    return
  }

  for (const url of uniqueUrls) {
    postDownloadRequest({
      id: crypto.randomUUID(),
      url,
      referrer: tab.url,
      cookies: await cookieString(url),
      method: "GET",
      timestamp: Date.now()
    })
  }
  lastError = null
}

async function harvestTabLinks(tabId: number): Promise<string[]> {
  try {
    // Firefox MV2 uses tabs.executeScript; Chrome MV3 uses scripting.executeScript
    if (typeof chrome.scripting !== "undefined") {
      const results = await chrome.scripting.executeScript({
        target: { tabId },
        func: () => {
          const anchors = Array.from(document.querySelectorAll<HTMLAnchorElement>("a[href]"))
          return anchors
            .map((anchor) => anchor.href)
            .filter((href) => href.startsWith("http://") || href.startsWith("https://"))
        }
      })
      return results.flatMap((result) => result.result ?? [])
    } else {
      // Firefox MV2 path
      const results = await browser.tabs.executeScript(tabId, {
        code: `(function() {
          var anchors = Array.from(document.querySelectorAll('a[href]'));
          return anchors.map(function(a) { return a.href; })
            .filter(function(h) { return h.startsWith('http://') || h.startsWith('https://'); });
        })()`
      })
      return (results[0] as string[] | undefined) ?? []
    }
  } catch (error) {
    lastError = error instanceof Error ? error.message : String(error)
    return []
  }
}

function postDownloadRequest(request: DownloadRequest): void {
  trace(`DownloadReq ${request.url.slice(0, 90)}`)
  postHostMessage({ DownloadReq: request })
}

function createContextMenu(): void {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: CONTEXT_MENU_ID,
      title: "Download with Ferric",
      contexts: ["link", "video", "audio", "image"]
    })
    chrome.contextMenus.create({
      id: CONTEXT_MENU_DOWNLOAD_ALL_ID,
      title: "Download All Links with Ferric",
      contexts: ["page", "selection"]
    })
  })
}

/** True when the user held Insert while clicking this link (capture even if the policy says no). */
async function userForcedCapture(url: string, originalUrl: string): Promise<boolean> {
  const click = await recentPreClick()
  return Boolean(click?.force && (click.href === url || click.href === originalUrl))
}

async function recentPreClick(): Promise<{ href: string; skip?: boolean; force?: boolean } | undefined> {
  const sessionStore = chrome.storage.session ?? chrome.storage.local
  const stored = (await sessionStore.get("ferricPreClick")) as {
    ferricPreClick?: { href: string; skip?: boolean; force?: boolean; timestamp: number }
  }
  const click = stored.ferricPreClick
  return click && Date.now() - click.timestamp < 5000 ? click : undefined
}

/** Sent by the in-page "Download with Ferric" button. */
async function downloadMedia(message: Extract<PopupMessage, { type: "download_media" }>): Promise<void> {
  const policy = await loadCurrentCapturePolicy()
  if (!policy.enabled || !isHttpUrl(message.url) || !daemonConnected) {
    lastError = daemonConnected ? lastError : "Ferric is not running"
    return
  }
  postDownloadRequest({
    id: crypto.randomUUID(),
    url: message.url,
    referrer: message.referrer,
    cookies: await cookieString(message.url),
    headers: { "user-agent": navigator.userAgent },
    mime_type: message.mime ?? manifestMime(message.url),
    method: "GET",
    timestamp: Date.now()
  })
}

/** True when the user Alt+clicked this link a moment ago (browser should keep the download). */
async function userSkippedCapture(url: string, originalUrl: string): Promise<boolean> {
  const click = await recentPreClick()
  return Boolean(click?.skip && (click.href === url || click.href === originalUrl))
}

// Small ring buffer of grab steps, readable from the service-worker console for diagnosis.
const grabTrace: string[] = []
;(globalThis as { __ferricTrace?: string[] }).__ferricTrace = grabTrace
function trace(message: string): void {
  grabTrace.push(`${new Date().toISOString().slice(11, 23)} ${message}`)
  if (grabTrace.length > 100) grabTrace.shift()
}

// Promise style (no callbacks): works on Chrome MV3 and, through firefox-compat, on Firefox.
async function settle(label: string, call: Promise<unknown>): Promise<void> {
  try {
    await call
    trace(`${label}: ok`)
  } catch (error) {
    trace(`${label}: ${error instanceof Error ? error.message : String(error)}`)
  }
}

function pauseChromeDownload(id: number): Promise<void> {
  return settle(`pause ${id}`, chrome.downloads.pause(id))
}

function resumeChromeDownload(id: number): Promise<void> {
  return settle(`resume ${id}`, chrome.downloads.resume(id))
}

function cancelChromeDownload(id: number): Promise<void> {
  return settle(`cancel ${id}`, chrome.downloads.cancel(id))
}

function eraseChromeDownload(id: number): Promise<void> {
  return settle(`erase ${id}`, chrome.downloads.erase({ id }))
}

async function getCookies(url: string): Promise<chrome.cookies.Cookie[]> {
  return (await chrome.cookies.getAll({ url })) ?? []
}

async function cookieString(url: string): Promise<string | undefined> {
  try {
    const cookies = await getCookies(url)
    const value = cookies.map((cookie) => `${cookie.name}=${cookie.value}`).join("; ")
    return value.length > 0 ? value : undefined
  } catch (error) {
    lastError = error instanceof Error ? error.message : String(error)
    return undefined
  }
}

function basename(path?: string): string | undefined {
  return path?.split(/[\\/]/).pop() || undefined
}

function isHttpUrl(url: string): boolean {
  return url.startsWith("http://") || url.startsWith("https://")
}

function manifestMime(url: string): string | undefined {
  const lower = url.toLowerCase().split("?")[0]
  if (lower.endsWith(".m3u8")) {
    return "application/vnd.apple.mpegurl"
  }
  if (lower.endsWith(".mpd")) {
    return "application/dash+xml"
  }
  return undefined
}
