// Drives Chromium with the Ferric extension loaded: opens the test page and clicks one link.
// Uses puppeteer-core because Playwright takes over download handling and hides the browser's real behaviour.
// Prints one JSON line per event. usage: node capture.mjs CHROME EXTENSION_DIR USER_DATA_DIR DOWNLOADS_DIR PAGE_URL LINK_ID
import puppeteer from "puppeteer-core"
import fs from "node:fs"
import path from "node:path"

const [chrome, ext, userData, dlDir, page, link] = process.argv.slice(2)
const say = (o) => console.log(JSON.stringify(o))
// Where the browser itself saves a download it keeps.
fs.mkdirSync(path.join(userData, "Default"), { recursive: true })
fs.writeFileSync(path.join(userData, "Default", "Preferences"),
  JSON.stringify({ download: { default_directory: dlDir, prompt_for_download: false }, savefile: { default_directory: dlDir } }))

const browser = await puppeteer.launch({
  executablePath: chrome,
  headless: "new", // new headless mode runs extensions and needs no display
  userDataDir: userData,
  pipe: true,
  ignoreDefaultArgs: ["--disable-extensions"],
  args: [`--disable-extensions-except=${ext}`, `--load-extension=${ext}`, "--no-first-run", "--no-sandbox"],
})
const target = await browser.waitForTarget((t) => t.type() === "service_worker" && t.url().includes("background"), { timeout: 20000 })
const worker = await target.worker()
say({ ev: "service-worker", url: target.url() })
await new Promise((r) => setTimeout(r, 6000)) // let the extension start and talk to the host
const p = await browser.newPage()
say({ ev: "step", n: "page opened" })
await p.goto(page)
say({ ev: "step", n: "page loaded" })
await p.click(`#${link}`)
say({ ev: "step", n: "clicked" })
await new Promise((r) => setTimeout(r, Number(process.env.WAIT_MS ?? 15000)))
const within = (ms, pr) => Promise.race([pr, new Promise((_, rej) => setTimeout(() => rej(new Error("timed out after " + ms + " ms")), ms))])
try { say({ ev: "ext-trace", lines: await within(5000, worker.evaluate(() => globalThis.__ferricTrace ?? [])) }) } catch (e) { say({ ev: "ext-trace-failed", error: String(e).slice(0, 120) }) }
say({ ev: "browser-files", files: fs.readdirSync(dlDir) })
say({ ev: "step", n: "closing" })
await Promise.race([browser.close(), new Promise((r) => setTimeout(r, 8000))])
process.exit(0) // the native host Chrome started can keep node alive otherwise
