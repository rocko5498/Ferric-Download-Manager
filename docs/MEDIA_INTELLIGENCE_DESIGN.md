# Media Intelligence Design

Ferric should handle legal, non-DRM media downloads transparently. It should not bypass DRM, protected streams, paywalls, or access controls.

## Scope

Supported:
- HLS (`.m3u8`) and DASH (`.mpd`) detection.
- Browser-observed media candidates.
- Quality/variant selection for accessible streams.
- Audio/video merge for legal non-DRM streams with user consent.
- Clear diagnostics for unsupported/protected streams.

Not supported:
- DRM bypass.
- Signature/cipher circumvention.
- Cookie DB scraping.
- Silent capture of protected/private streams.

## Media Sniffer

Extension-side signals:
- `Content-Type`: `video/*`, `audio/*`, `application/vnd.apple.mpegurl`, `application/x-mpegurl`, `application/dash+xml`.
- URL patterns: `.m3u8`, `.mpd`, media segment patterns, large range video/audio requests.
- Page context: top-frame URL, referrer, initiator, tab title.

Daemon-side validation:
- Probe candidate URL with safe request path.
- Validate URL through `security::validate_url()`.
- Never log cookies/headers.
- Classify candidate as `direct_file`, `hls_manifest`, `dash_manifest`, `media_segment`, `unsupported`, or `protected_or_expired`.

UI:
- "Media found" panel separate from normal file capture.
- Show source page, detected type, estimated size/quality if known, and safety/privacy notes.
- Offer only supported legal/non-DRM actions.

## HLS / DASH Quality Selection

Current state:
- First-pass HLS/DASH support exists.
- DASH parser chooses one highest-bandwidth representation.

Target:
- Parse master playlists / adaptation sets into variants.
- Show quality, bandwidth, codec, resolution, audio language, subtitles if available.
- Allow user to choose:
  - best video + best audio
  - specific resolution
  - audio-only where available
  - subtitles where available

Acceptance:
- UI displays at least 2 variants from a test manifest.
- User choice is passed to daemon before download starts.
- Unsupported manifest forms produce actionable explanations.

## ffmpeg Merge Path

Goal:
- Merge separate audio/video streams after download when legal/non-DRM and accessible.

Design:
- `ffmpeg` is optional external dependency.
- Detect `ffmpeg` availability.
- If missing, show "download streams separately" or "configure ffmpeg."
- Never run arbitrary shell strings; invoke executable with argument array.
- Sanitize all paths and write only inside selected save directory.
- Show merge progress and keep source segments until merge succeeds.

Acceptance:
- Test merge command construction without shell interpolation.
- UI shows merge state: pending, merging, complete, failed.

## Explanations

Ferric should explain:
- Why media was not captured.
- Why a stream cannot resume.
- Why a URL expired.
- Whether slowdown appears server/CDN-side, queue-side, connection-side, or local disk-side.
- Whether cookies/headers are required, without displaying them.

