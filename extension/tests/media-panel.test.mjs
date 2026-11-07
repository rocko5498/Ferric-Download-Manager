import assert from "node:assert/strict"
import test from "node:test"
import { build } from "esbuild"

const out = await build({ entryPoints: ["media-panel.ts"], bundle: true, format: "esm", write: false, platform: "neutral" })
const { formatSize, panelTitle, hitLabel, formatLabel, pickMediaElement, codecName, visibleFraction } = await import(
  `data:text/javascript;base64,${Buffer.from(out.outputFiles[0].text).toString("base64")}`
)

test("sizes: bytes, then two decimals with the unit", () => {
  assert.equal(formatSize(512, "en-US"), "512 Bytes")
  assert.equal(formatSize(2048, "en-US"), "2.00 KB")
  assert.equal(formatSize(12.34 * 1024 * 1024, "en-US"), "12.34 MB")
  assert.equal(formatSize(3 * 1024 ** 3, "en-US"), "3.00 GB")
  assert.equal(formatSize(1.5 * 1024 * 1024, "de-DE"), "1,50 MB")
})

test("title wording matches IDM", () => {
  assert.equal(panelTitle("video"), "Download this video")
  assert.equal(panelTitle("audio"), "Download this audio")
})

test("entry labels show kind and size", () => {
  assert.equal(hitLabel({ url: "u", kind: "direct", type: "video", ext: "mp4", size: 5 * 1024 * 1024 }, "en-US"), "MP4  5.00 MB")
  assert.equal(hitLabel({ url: "u", kind: "hls", type: "video", ext: "m3u8" }, "en-US"), "HLS stream")
  assert.equal(formatLabel({ id: "137", ext: "mp4", height: 1080, fps: 60, size: 40 * 1024 * 1024 }, "en-US"), "1080p60 MP4  40.00 MB")
  assert.equal(formatLabel({ id: "251", ext: "webm", audio_only: true, abr: 129.4 }, "en-US"), "Audio 129k WEBM")
  assert.equal(formatLabel({ id: "22", ext: "mp4", height: 720, fps: 30 }, "en-US"), "720p MP4")
})

test("the hovered element wins, otherwise the largest sane one; slivers and tiny ones are skipped", () => {
  const big = { w: 1280, h: 720, visible: true }
  const small = { w: 320, h: 180, visible: true }
  assert.equal(pickMediaElement([small, big]), 1)
  assert.equal(pickMediaElement([small, { ...big, hovered: false }, { ...small, hovered: true }]), 2)
  assert.equal(pickMediaElement([{ w: 1000, h: 60, visible: true }]), -1)
  assert.equal(pickMediaElement([{ w: 100, h: 60, visible: true }]), -1)
  assert.equal(pickMediaElement([{ ...big, visible: false }]), -1)
})

test("rows that differ only by codec or bitrate can be told apart", () => {
  assert.equal(codecName("avc1.640028"), "H.264")
  assert.equal(codecName("vp09.00.40.08"), "VP9")
  assert.equal(codecName("av01.0.12M.08"), "AV1")
  assert.equal(codecName("hvc1.2.4.L93"), "H.265")
  assert.equal(codecName("video"), "")
  assert.equal(formatLabel({ id: "a", ext: "mp4", height: 1080, vcodec: "avc1.640028", size: 200 * 1024 * 1024 }, "en-US"), "1080p MP4 H.264  200.00 MB")
  assert.equal(formatLabel({ id: "b", ext: "webm", height: 1080, vcodec: "vp09.00.40.08", size: 360 * 1024 * 1024 }, "en-US"), "1080p WEBM VP9  360.00 MB")
  // a playlist variant has no size: bitrate stands in, and the container reads HLS, not M3U8
  assert.equal(formatLabel({ id: "h", ext: "m3u8", height: 1080, tbr: 8001, vcodec: "avc1.64002a" }, "en-US"), "1080p HLS H.264  8.0 Mbps")
  assert.equal(formatLabel({ id: "h2", ext: "m3u8", height: 270, tbr: 541, vcodec: "avc1.640015" }, "en-US"), "270p HLS H.264  541 kbps")
})

test("a video parked above the viewport (YouTube before the first play) does not count as visible", () => {
  const box = (top, height) => ({ left: 0, top, right: 914, bottom: top + height, width: 914, height })
  assert.equal(visibleFraction(box(55, 523), 929, 867), 1)
  assert.ok(visibleFraction(box(-467, 523), 929, 867) < 0.2, "only 56 of 523 px on screen")
  assert.ok(visibleFraction(box(-200, 523), 929, 867) > 0.2, "most of it still showing")
  assert.equal(visibleFraction(box(900, 300), 929, 867), 0)
  assert.equal(visibleFraction({ left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 }, 929, 867), 0)
})
