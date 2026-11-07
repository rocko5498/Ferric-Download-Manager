import test from "node:test"
import assert from "node:assert/strict"
import { DEFAULT_CAPTURE_POLICY as P, shouldCaptureDownloadItem as cap } from "../capture-policy.ts"

const item = (o) => ({ url: "https://x.example/a", fileSize: 0, mime: "", filename: "", ...o })

test("small explicit download with known extension is captured (IDM has no size floor)", () => {
  assert.equal(cap(item({ url: "https://x.example/a.zip", fileSize: 200_000 }), P), true)
})
test("signed link with token query is not blocked as auth when it has a known extension", () => {
  assert.equal(cap(item({ url: "https://x.example/f.iso?token=abc&code=1" }), P), true)
})
test("extensionless auth redirect is still rejected", () => {
  assert.equal(cap(item({ url: "https://x.example/x?code=1", mime: "application/octet-stream" }), P), false)
})
test("html and json are never captured", () => {
  assert.equal(cap(item({ url: "https://x.example/a.zip", mime: "text/html" }), P), false)
})
test("a bare application/* mime without a listed extension stays with the browser (IDM captures listed types only)", () => {
  assert.equal(cap(item({ url: "https://x.example/get?id=5", mime: "application/x-foo", fileSize: 5_000_000 }), P), false)
  assert.equal(cap(item({ url: "https://x.example/data.dat", mime: "application/octet-stream", fileSize: 5_000_000 }), P), false)
})
test("media mime types are still captured without an extension", () => {
  assert.equal(cap(item({ url: "https://x.example/get?id=5", mime: "video/mp4", fileSize: 5_000_000 }), P), true)
})
test("tiny extensionless mime-only item stays below the floor", () => {
  assert.equal(cap(item({ url: "https://x.example/get?id=5", mime: "application/x-foo", fileSize: 1000 }), P), false)
})

test("IDM seeded exclusions and extra default extensions are active", () => {
  assert.equal(cap(item({ url: "https://web.whatsapp.com/res/ding.mp3", fileSize: 9_000_000 }), P), false)
  assert.equal(cap(item({ url: "https://x.example/disk.rmvb" }), P), true)
})

test("capture list is IDM's own, with R0*/R1* prefix entries; types outside it stay with the browser", () => {
  assert.equal(cap(item({ url: "https://x.example/archive.r07" }), P), true)
  assert.equal(cap(item({ url: "https://x.example/archive.r23" }), P), false)
  assert.equal(cap(item({ url: "https://x.example/clip.qt" }), P), true)
  assert.equal(cap(item({ url: "https://x.example/report.docx" }), P), false)
})
