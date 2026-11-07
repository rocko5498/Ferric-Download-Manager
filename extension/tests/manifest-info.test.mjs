import assert from "node:assert/strict"
import test from "node:test"
import { build } from "esbuild"

async function load(entry) {
  const out = await build({ entryPoints: [entry], bundle: true, format: "esm", write: false, platform: "neutral" })
  return import(`data:text/javascript;base64,${Buffer.from(out.outputFiles[0].text).toString("base64")}`)
}
const { parseManifest, groupManifests } = await load("manifest-info.ts")
const { hitLabel, bestHit } = await load("media-panel.ts")

const BASE = "https://cdn.example.com/video/master.m3u8"
const MASTER = `#EXTM3U
#EXT-X-VERSION:6
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="aud",NAME="en",DEFAULT=YES,URI="audio/en.m3u8"
#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID="sub",NAME="en",URI="subs/en.m3u8"
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,AUDIO="aud"
v1080/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2800000,RESOLUTION=1280x720,AUDIO="aud"
v720/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1200000,RESOLUTION=854x480,AUDIO="aud"
v480/index.m3u8
`
const MEDIA = `#EXTM3U
#EXT-X-TARGETDURATION:6
#EXTINF:6.0,
seg0.ts
#EXTINF:6.0,
seg1.ts
#EXT-X-ENDLIST
`

test("a master playlist lists its qualities and the playlists it points to", () => {
  const info = parseManifest(MASTER, BASE)
  assert.equal(info.role, "master")
  assert.deepEqual(info.qualities, [1080, 720, 480])
  assert.ok(info.children.includes("https://cdn.example.com/video/v720/index.m3u8"))
  assert.ok(info.children.includes("https://cdn.example.com/video/audio/en.m3u8"), "audio rendition is a child, not a video")
  assert.ok(info.children.includes("https://cdn.example.com/video/subs/en.m3u8"), "subtitle rendition too")
})

test("a media playlist (segments) is recognised and has no children", () => {
  const info = parseManifest(MEDIA, "https://cdn.example.com/video/v720/index.m3u8")
  assert.equal(info.role, "media")
  assert.deepEqual(info.children, [])
})

test("DASH manifests list their heights; junk is unknown", () => {
  const mpd = `<?xml version="1.0"?><MPD xmlns="urn:mpeg:dash:schema:mpd:2011"><Period><AdaptationSet>
    <Representation id="a" height="1080" bandwidth="4000000"/><Representation id="b" height="720" bandwidth="2000000"/>
    <Representation id="c" height="720" bandwidth="1900000"/></AdaptationSet></Period></MPD>`
  const info = parseManifest(mpd, "https://x.test/s.mpd")
  assert.equal(info.role, "master")
  assert.deepEqual(info.qualities, [1080, 720])
  assert.equal(parseManifest("<html>Forbidden</html>", "https://x.test/p.m3u8").role, "unknown")
})

const hit = (url, extra = {}) => ({ url, kind: "hls", type: "video", ext: "m3u8", ...extra })

test("one video loaded as master + two variant playlists is ONE entry (the repeated 'HLS stream' bug)", () => {
  const master = parseManifest(MASTER, BASE)
  // newest first, like the tracker keeps them
  const hits = [
    hit("https://cdn.example.com/video/v480/index.m3u8", { info: parseManifest(MEDIA, "https://cdn.example.com/video/v480/index.m3u8") }),
    hit("https://cdn.example.com/video/v720/index.m3u8", { info: parseManifest(MEDIA, "https://cdn.example.com/video/v720/index.m3u8") }),
    hit(BASE, { info: master })
  ]
  const grouped = groupManifests(hits)
  assert.equal(grouped.length, 1)
  assert.equal(grouped[0].url, BASE)
})

test("unreadable playlists: the first one requested stands for the stream", () => {
  const hits = [hit("https://x.test/c.m3u8"), hit("https://x.test/b.m3u8"), hit("https://x.test/a.m3u8")]
  assert.deepEqual(groupManifests(hits).map((h) => h.url), ["https://x.test/a.m3u8"])
})

test("direct files and unrelated videos are left alone", () => {
  const direct = { url: "https://x.test/clip.mp4", kind: "direct", type: "video", ext: "mp4", size: 5_000_000 }
  const otherMaster = hit("https://other.test/master.m3u8", { info: { role: "master", qualities: [720], children: [] } })
  const grouped = groupManifests([direct, otherMaster, hit(BASE, { info: parseManifest(MASTER, BASE) })])
  assert.equal(grouped.length, 3)
})

test("labels name the qualities; the main button prefers a master over a stub or a lone quality", () => {
  const master = hit(BASE, { info: parseManifest(MASTER, BASE) })
  assert.equal(hitLabel(master, "en-US"), "HLS stream (1080p, 720p, 480p)")
  const lone = hit("https://x.test/v.m3u8", { info: { role: "media", qualities: [], children: [] } })
  assert.equal(hitLabel(lone, "en-US"), "HLS stream (single quality)")
  assert.equal(hitLabel(hit("https://x.test/u.m3u8"), "en-US"), "HLS stream")
  const direct = { url: "https://x.test/clip.mp4", kind: "direct", type: "video", ext: "mp4", size: 5_000_000 }
  assert.equal(bestHit([lone, direct, master]).url, BASE)
  assert.equal(bestHit([lone, direct]).url, direct.url)
  assert.equal(bestHit([]), undefined)
})
