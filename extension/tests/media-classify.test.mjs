import assert from "node:assert/strict"
import test from "node:test"
import { build } from "esbuild"

async function load(entry) {
  const out = await build({ entryPoints: [entry], bundle: true, format: "esm", write: false, platform: "neutral" })
  return import(`data:text/javascript;base64,${Buffer.from(out.outputFiles[0].text).toString("base64")}`)
}
const { classifyResponse, isPageExtractable, isStreamChunk } = await load("media-classify.ts")
const { DEFAULT_CAPTURE_POLICY: policy } = await load("capture-policy.ts")

const resp = (url, headers = {}, status = 200, pageUrl) => ({ url, status, headers, pageUrl })

test("plain mp4 response is offered with its size", () => {
  const hit = classifyResponse(resp("https://cdn.test/v/clip.mp4", { "content-type": "video/mp4", "content-length": "5000000" }), policy)
  assert.equal(hit.kind, "direct")
  assert.equal(hit.type, "video")
  assert.equal(hit.size, 5000000)
})

test("range response reports the total size from content-range", () => {
  const hit = classifyResponse(resp("https://cdn.test/clip.mp4", { "content-type": "video/mp4", "content-range": "bytes 0-1023/987654" }, 206), policy)
  assert.equal(hit.size, 987654)
})

test("hls and dash manifests are offered as manifests", () => {
  assert.equal(classifyResponse(resp("https://cdn.test/live/master.m3u8", { "content-type": "application/vnd.apple.mpegurl" }), policy).kind, "hls")
  assert.equal(classifyResponse(resp("https://cdn.test/stream.mpd", { "content-type": "application/dash+xml" }), policy).kind, "dash")
})

test("stream pieces are never offered", () => {
  assert.equal(classifyResponse(resp("https://cdn.test/seg/1.ts", { "content-type": "video/mp2t" }), policy), null)
  assert.equal(classifyResponse(resp("https://cdn.test/seg/1.m4s", { "content-type": "video/iso.segment" }), policy), null)
  const yt = "https://rr1---sn-x.googlevideo.com/videoplayback?itag=137&range=0-99"
  assert.equal(classifyResponse(resp(yt, { "content-type": "video/mp4" }), policy), null)
  assert.equal(isStreamChunk(yt), true)
})

test("html, images and error statuses are ignored", () => {
  assert.equal(classifyResponse(resp("https://x.test/", { "content-type": "text/html" }), policy), null)
  assert.equal(classifyResponse(resp("https://x.test/a.gif", { "content-type": "image/gif" }), policy), null)
  assert.equal(classifyResponse(resp("https://x.test/a.mp4", { "content-type": "video/mp4" }, 404), policy), null)
})

test("minimum sizes: floors for mp3/ogg, 100 KB for video types", () => {
  assert.equal(classifyResponse(resp("https://x.test/ding.mp3", { "content-type": "audio/mpeg", "content-length": "20000" }), policy), null)
  assert.ok(classifyResponse(resp("https://x.test/song.mp3", { "content-type": "audio/mpeg", "content-length": "6000000" }), policy))
  assert.equal(classifyResponse(resp("https://x.test/a.ogg", { "content-type": "audio/ogg", "content-length": "50000" }), policy), null)
  assert.equal(classifyResponse(resp("https://x.test/stub.mp4", { "content-type": "video/mp4", "content-length": "1210" }), policy), null, "ad/tracking stubs are not videos")
  assert.ok(classifyResponse(resp("https://x.test/clip.mp4", { "content-type": "video/mp4", "content-length": "250000" }), policy))
  assert.ok(classifyResponse(resp("https://x.test/live.mp4", { "content-type": "video/mp4" }), policy), "unknown size is still offered")
})

test("panel-excluded sites (gif/animation CDNs) never get a panel", () => {
  assert.equal(classifyResponse(resp("https://media.tenor.co/x/clip.mp4", { "content-type": "video/mp4" }), policy), null)
  assert.equal(classifyResponse(resp("https://media3.giphy.com/media/x/giphy.mp4", { "content-type": "video/mp4" }), policy), null)
  assert.equal(classifyResponse(resp("https://cdn.test/clip.mp4", { "content-type": "video/mp4" }, 200, "https://static.gstatic.com/x"), policy), null)
})

test("audio files are labelled audio; type comes from the mime table when the name has no extension", () => {
  assert.equal(classifyResponse(resp("https://x.test/stream?id=9", { "content-type": "audio/mpeg", "content-length": "3000000" }), policy).type, "audio")
  const hit = classifyResponse(resp("https://x.test/get?id=9", { "content-type": "video/webm", "content-length": "3000000" }), policy)
  assert.equal(hit.ext, "webm")
})

test("pages the extractor can handle are recognised from the URL", () => {
  assert.equal(isPageExtractable("https://www.youtube.com/watch?v=N-xHcvug3WI"), true)
  assert.equal(isPageExtractable("https://www.youtube.com/watch?list=PL1&v=N-xHcvug3WI"), true)
  assert.equal(isPageExtractable("https://www.youtube.com/shorts/abcDEF12345"), true)
  assert.equal(isPageExtractable("https://youtu.be/N-xHcvug3WI"), true)
  assert.equal(isPageExtractable("https://vimeo.com/76979871"), true)
  assert.equal(isPageExtractable("https://www.youtube.com/"), false)
  assert.equal(isPageExtractable("https://example.com/watch?v=1"), false)
})
