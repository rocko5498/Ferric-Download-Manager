import assert from "node:assert/strict"
import test from "node:test"
import { build } from "esbuild"

const out = await build({ entryPoints: ["grab.ts"], bundle: true, format: "esm", write: false, platform: "neutral" })
const mod = await import(`data:text/javascript;base64,${Buffer.from(out.outputFiles[0].text).toString("base64")}`)
const { decideGrab } = mod
const { DEFAULT_CAPTURE_POLICY: policy } = await import(
  `data:text/javascript;base64,${Buffer.from((await build({ entryPoints: ["capture-policy.ts"], bundle: true, format: "esm", write: false, platform: "neutral" })).outputFiles[0].text).toString("base64")}`
)

const base = { forced: false, skipped: false }

test("zip with attachment name is captured", () => {
  const d = decideGrab({ ...base, url: "https://x.test/dl?id=1", filename: "Hello-World-master.zip", mime: "application/zip" }, policy)
  assert.equal(d.capture, true)
})
test("zip recognised by type alone when name is unhelpful", () => {
  const d = decideGrab({ ...base, url: "https://x.test/dl?id=1", filename: "", mime: "application/zip" }, policy)
  assert.equal(d.capture, true)
})
test("html page is not captured", () => {
  assert.equal(decideGrab({ ...base, url: "https://x.test/", filename: "index.html", mime: "text/html" }, policy).capture, false)
})
test("Alt/skip keeps the browser download", () => {
  assert.equal(decideGrab({ ...base, skipped: true, url: "https://x.test/a.zip", filename: "a.zip" }, policy).capture, false)
})
test("Insert/force captures anything", () => {
  assert.equal(decideGrab({ ...base, forced: true, url: "https://x.test/a.dat", filename: "a.dat" }, policy).capture, true)
})
test("excluded url is not captured", () => {
  assert.equal(decideGrab({ ...base, url: "https://download.windowsupdate.com/x.cab", filename: "x.cab" }, policy).capture, false)
})
test("non-http is ignored", () => {
  assert.equal(decideGrab({ ...base, url: "blob:https://x.test/1", filename: "a.zip" }, policy).capture, false)
})

test("IDM's never-capture types stay with the browser", () => {
  for (const [url, filename, mime] of [
    ["https://x.test/app.js", "app.js", "application/javascript"],
    ["https://x.test/data.json", "data.json", "application/json"],
    ["https://x.test/logo.png", "logo.png", "image/png"],
    ["https://x.test/a/b-internal-c/d", "d.zip", "application/zip"]
  ]) {
    assert.equal(decideGrab({ ...base, url, filename, mime }, policy).capture, false, url)
  }
})
