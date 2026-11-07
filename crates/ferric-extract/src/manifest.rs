use ferric_common::ExtractedFormat;
use quick_xml::{events::Event, Reader};

pub async fn expand_manifest_formats(client: &reqwest::Client, formats: &mut Vec<ExtractedFormat>) {
    let manifests: Vec<(String, String)> = formats
        .iter()
        .filter_map(|format| match format.protocol.as_deref() {
            Some("m3u8" | "dash") => format
                .manifest_url
                .as_ref()
                .or(Some(&format.url))
                .map(|url| (format.protocol.clone().unwrap_or_default(), url.clone())),
            _ => None,
        })
        .collect();

    for (protocol, url) in manifests {
        let Ok(resp) = client.get(&url).send().await else {
            continue;
        };
        let Ok(resp) = resp.error_for_status() else {
            continue;
        };
        let Ok(bytes) = resp.bytes().await else {
            continue;
        };
        let mut expanded = if protocol == "m3u8" {
            parse_hls_formats(&url, &bytes)
        } else {
            std::str::from_utf8(&bytes)
                .map(|manifest| parse_dash_formats(&url, manifest))
                .unwrap_or_default()
        };
        formats.append(&mut expanded);
    }
}

fn parse_hls_formats(manifest_url: &str, bytes: &[u8]) -> Vec<ExtractedFormat> {
    let Ok(playlist) = m3u8_rs::parse_playlist_res(bytes) else {
        return Vec::new();
    };
    let m3u8_rs::Playlist::MasterPlaylist(master) = playlist else {
        return Vec::new();
    };
    if has_marker(bytes, b"#EXT-X-KEY") || has_marker(bytes, b"#EXT-X-SESSION-KEY") {
        return Vec::new();
    }

    master
        .variants
        .iter()
        .enumerate()
        // Trick-play (I-frame) playlists only serve scrubbing; they are not a quality to download.
        .filter(|(_, variant)| !variant.is_i_frame)
        .filter_map(|(index, variant)| {
            let url = resolve_url(manifest_url, &variant.uri)?;
            let width = variant
                .resolution
                .as_ref()
                .and_then(|resolution| u32::try_from(resolution.width).ok());
            let height = variant
                .resolution
                .as_ref()
                .and_then(|resolution| u32::try_from(resolution.height).ok());
            let kbps = Some(variant.bandwidth as f64 / 1000.0);
            // A variant that names an AUDIO group carries video only; its sound is a separate playlist.
            let separate_audio = variant.audio.is_some();
            let codecs: Vec<&str> = variant
                .codecs
                .as_deref()
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .collect();
            let vcodec = codecs
                .iter()
                .find(|c| is_video_codec(c))
                .map_or_else(|| "video".to_string(), |c| (*c).to_string());
            let acodec = if separate_audio {
                "none".to_string()
            } else {
                codecs
                    .iter()
                    .find(|c| !is_video_codec(c))
                    .map_or_else(|| "audio".to_string(), |c| (*c).to_string())
            };
            Some(ExtractedFormat {
                id: format!("hls-{index}"),
                label: variant_label("HLS", width, height, kbps, variant.codecs.as_deref()),
                url,
                ext: "m3u8".to_string(),
                protocol: Some("m3u8".to_string()),
                manifest_url: Some(manifest_url.to_string()),
                format_note: Some(
                    variant
                        .audio
                        .as_ref()
                        .map_or_else(|| "HLS variant".to_string(), |g| format!("audio-group:{g}")),
                ),
                width,
                height,
                tbr_kbps: kbps,
                abr_kbps: None,
                vbr_kbps: kbps,
                fps: None,
                vcodec: Some(vcodec),
                acodec: Some(acodec),
                container: None,
                language: None,
                filesize: None,
                has_drm: false,
                headers: Vec::new(),
                is_audio_only: false,
            })
        })
        .chain(master.alternatives.iter().enumerate().filter_map(|(index, alt)| {
            if !matches!(alt.media_type, m3u8_rs::AlternativeMediaType::Audio) {
                return None;
            }
            let url = resolve_url(manifest_url, alt.uri.as_deref()?)?;
            // The default track wins when the player pairs audio with a video variant.
            let abr = if alt.default { 128.0 } else { 96.0 };
            Some(ExtractedFormat {
                id: format!("hls-audio-{index}"),
                label: format!("HLS audio {}", alt.name),
                url,
                ext: "m3u8".to_string(),
                protocol: Some("m3u8".to_string()),
                manifest_url: Some(manifest_url.to_string()),
                format_note: Some(format!("audio-group:{}", alt.group_id)),
                width: None,
                height: None,
                tbr_kbps: Some(abr),
                abr_kbps: Some(abr),
                vbr_kbps: None,
                fps: None,
                vcodec: Some("none".to_string()),
                acodec: Some("audio".to_string()),
                container: None,
                language: alt.language.clone(),
                filesize: None,
                has_drm: false,
                headers: Vec::new(),
                is_audio_only: true,
            })
        }))
        .collect()
}

fn is_video_codec(c: &str) -> bool {
    ["avc", "hvc", "hev", "vp", "av01"]
        .iter()
        .any(|p| c.starts_with(p))
}

fn has_marker(bytes: &[u8], marker: &[u8]) -> bool {
    bytes.windows(marker.len()).any(|w| w == marker)
}

/// True for a playlist whose segments use anything but plain AES-128 (SAMPLE-AES and key systems such
/// as Widevine, PlayReady and FairPlay are protected streams, which Ferric never offers).
fn media_playlist_is_protected(text: &str) -> bool {
    text.lines()
        .filter(|l| l.starts_with("#EXT-X-KEY") || l.starts_with("#EXT-X-SESSION-KEY"))
        .any(|l| {
            let upper = l.to_ascii_uppercase();
            let method_ok = upper.contains("METHOD=NONE") || upper.contains("METHOD=AES-128");
            let keyformat_ok =
                !upper.contains("KEYFORMAT=") || upper.contains("KEYFORMAT=\"IDENTITY\"");
            !(method_ok && keyformat_ok)
        })
}

/// The qualities of an address that is itself a playlist or manifest (no site extractor applies):
/// every HLS variant (with its audio when it is a separate group), one entry for a single-quality
/// playlist, one "best" entry for DASH. `None` when it is not a manifest or is protected.
pub async fn formats_from_manifest_url(
    client: &reqwest::Client,
    url: &str,
) -> Option<Vec<ExtractedFormat>> {
    let resp = client
        .get(url)
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let bytes = resp.bytes().await.ok()?;
    if bytes.len() > 4 * 1024 * 1024 {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes);
    if text.trim_start().starts_with("#EXTM3U") {
        if media_playlist_is_protected(&text) {
            return None;
        }
        let variants = parse_hls_formats(url, &bytes);
        if !variants.is_empty() {
            return Some(variants);
        }
        // Not a master: a playlist of segments for one quality.
        return text
            .contains("#EXTINF")
            .then(|| vec![single_format(url, "HLS single quality", "m3u8", "m3u8")]);
    }
    if text.contains("<MPD") {
        if text.contains("<ContentProtection") {
            return None;
        }
        return Some(vec![single_format(url, "DASH best quality", "mp4", "dash")]);
    }
    None
}

fn single_format(url: &str, label: &str, ext: &str, protocol: &str) -> ExtractedFormat {
    ExtractedFormat {
        id: format!("{protocol}-0"),
        label: label.to_string(),
        url: url.to_string(),
        ext: ext.to_string(),
        protocol: Some(protocol.to_string()),
        manifest_url: Some(url.to_string()),
        format_note: None,
        width: None,
        height: None,
        tbr_kbps: None,
        abr_kbps: None,
        vbr_kbps: None,
        fps: None,
        vcodec: Some("video".to_string()),
        acodec: Some("audio".to_string()),
        container: None,
        language: None,
        filesize: None,
        has_drm: false,
        headers: Vec::new(),
        is_audio_only: false,
    }
}

fn parse_dash_formats(manifest_url: &str, manifest: &str) -> Vec<ExtractedFormat> {
    if manifest.contains("<ContentProtection") {
        return Vec::new();
    }

    let mut reader = Reader::from_str(manifest);
    reader.config_mut().trim_text(true);
    let mut formats = Vec::new();
    let mut base_url = String::new();
    let mut in_base_url = false;
    let mut current_kind: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) if event.name().as_ref() == b"AdaptationSet" => {
                current_kind = attr(&event, b"contentType")
                    .or_else(|| attr(&event, b"mimeType"))
                    .map(|value| value.to_ascii_lowercase());
            }
            Ok(Event::End(event)) if event.name().as_ref() == b"AdaptationSet" => {
                current_kind = None;
            }
            Ok(Event::Start(event)) if event.name().as_ref() == b"BaseURL" => {
                in_base_url = true;
                base_url.clear();
            }
            Ok(Event::End(event)) if event.name().as_ref() == b"BaseURL" => {
                in_base_url = false;
            }
            Ok(Event::Text(text)) if in_base_url => {
                if let Ok(value) = text.decode() {
                    base_url.push_str(value.trim());
                }
            }
            Ok(Event::Start(event) | Event::Empty(event))
                if event.name().as_ref() == b"Representation" =>
            {
                let id = attr(&event, b"id").unwrap_or_else(|| format!("dash-{}", formats.len()));
                let bandwidth =
                    attr(&event, b"bandwidth").and_then(|value| value.parse::<u64>().ok());
                let width = attr(&event, b"width").and_then(|value| value.parse::<u32>().ok());
                let height = attr(&event, b"height").and_then(|value| value.parse::<u32>().ok());
                let codecs = attr(&event, b"codecs");
                let audio_only = current_kind
                    .as_deref()
                    .is_some_and(|kind| kind == "audio" || kind.starts_with("audio/"));
                let url = resolve_url(manifest_url, &base_url)
                    .unwrap_or_else(|| manifest_url.to_string());
                let kbps = bandwidth.map(|value| value as f64 / 1000.0);
                formats.push(ExtractedFormat {
                    id: format!("dash-{id}"),
                    label: variant_label("DASH", width, height, kbps, codecs.as_deref()),
                    url,
                    ext: "mp4".to_string(),
                    protocol: Some("dash".to_string()),
                    manifest_url: Some(manifest_url.to_string()),
                    format_note: Some(
                        if audio_only {
                            "DASH audio"
                        } else {
                            "DASH video"
                        }
                        .to_string(),
                    ),
                    width,
                    height,
                    tbr_kbps: kbps,
                    abr_kbps: audio_only.then_some(kbps).flatten(),
                    vbr_kbps: (!audio_only).then_some(kbps).flatten(),
                    fps: None,
                    vcodec: (!audio_only).then(|| codecs.clone()).flatten(),
                    acodec: audio_only.then_some(codecs).flatten(),
                    container: Some("mp4".to_string()),
                    language: None,
                    filesize: None,
                    has_drm: false,
                    headers: Vec::new(),
                    is_audio_only: audio_only,
                });
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    formats
}

fn attr(event: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    event
        .attributes()
        .flatten()
        .find(|attr| attr.key.as_ref() == name)
        .and_then(|attr| String::from_utf8(attr.value.into_owned()).ok())
}

fn resolve_url(base: &str, value: &str) -> Option<String> {
    url::Url::parse(base)
        .ok()
        .and_then(|base| base.join(value).ok())
        .map(|url| url.to_string())
}

fn variant_label(
    prefix: &str,
    width: Option<u32>,
    height: Option<u32>,
    kbps: Option<f64>,
    codecs: Option<&str>,
) -> String {
    let mut parts = vec![prefix.to_string()];
    if let Some(h) = height {
        parts.push(format!("{h}p"));
    } else if let (Some(w), Some(h)) = (width, height) {
        parts.push(format!("{w}x{h}"));
    }
    if let Some(kbps) = kbps {
        parts.push(format!("{kbps:.0} kbps"));
    }
    if let Some(codecs) = codecs.filter(|s| !s.is_empty()) {
        parts.push(codecs.to_string());
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hls_master_variants() {
        let bytes = br#"#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1280x720,CODECS="avc1,mp4a"
720/index.m3u8
"#;
        let formats = parse_hls_formats("https://example.com/master.m3u8", bytes);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].height, Some(720));
        assert_eq!(formats[0].url, "https://example.com/720/index.m3u8");
    }

    #[test]
    fn parses_dash_representations() {
        let manifest = r#"<MPD><Period><AdaptationSet contentType="video"><BaseURL>video/</BaseURL><Representation id="v1" bandwidth="3000000" width="1920" height="1080" codecs="avc1"/></AdaptationSet></Period></MPD>"#;
        let formats = parse_dash_formats("https://example.com/manifest.mpd", manifest);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].height, Some(1080));
        assert_eq!(formats[0].url, "https://example.com/video/");
    }

    #[test]
    fn pairs_a_video_only_variant_with_its_audio_group() {
        let bytes = br#"#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="aud",NAME="English",DEFAULT=YES,LANGUAGE="en",URI="audio/en.m3u8"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="aud",NAME="French",DEFAULT=NO,LANGUAGE="fr",URI="audio/fr.m3u8"
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,AUDIO="aud"
v1080/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=640x360
muxed360/index.m3u8
"#;
        let formats = parse_hls_formats("https://cdn.test/video/master.m3u8", bytes);
        let video: Vec<_> = formats.iter().filter(|f| !f.is_audio_only).collect();
        assert_eq!(video.len(), 2);
        assert_eq!(
            video[0].acodec.as_deref(),
            Some("none"),
            "1080p has its sound in a separate playlist"
        );
        assert_eq!(
            video[1].acodec.as_deref(),
            Some("audio"),
            "the 360p variant is already muxed"
        );
        let coded = parse_hls_formats(
            "https://cdn.test/m.m3u8",
            b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720,CODECS=\"avc1.64001f, mp4a.40.2\"\nv.m3u8\n",
        );
        assert_eq!(coded[0].vcodec.as_deref(), Some("avc1.64001f"));
        assert_eq!(coded[0].acodec.as_deref(), Some("mp4a.40.2"));
        let audio: Vec<_> = formats.iter().filter(|f| f.is_audio_only).collect();
        assert_eq!(audio.len(), 2);
        assert!(
            audio[0].abr_kbps > audio[1].abr_kbps,
            "the default track ranks first"
        );
        assert_eq!(audio[0].url, "https://cdn.test/video/audio/en.m3u8");
        assert_eq!(
            video[0].format_note, audio[0].format_note,
            "the variant and its audio share a group tag"
        );
        assert_eq!(video[1].format_note.as_deref(), Some("HLS variant"));
    }

    #[test]
    fn trick_play_playlists_are_not_a_quality() {
        let bytes = br#"#EXTM3U
#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=187000,RESOLUTION=1920x1080,URI="iframe/index.m3u8"
#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1280x720
720/index.m3u8
"#;
        let formats = parse_hls_formats("https://example.com/master.m3u8", bytes);
        assert_eq!(formats.len(), 1, "only the real 720p variant");
        assert_eq!(formats[0].height, Some(720));
    }

    #[test]
    fn encrypted_masters_and_non_aes_keys_are_not_offered() {
        let session = b"#EXTM3U\n#EXT-X-SESSION-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\"\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv.m3u8\n";
        assert!(parse_hls_formats("https://x.test/m.m3u8", session).is_empty());
        assert!(!media_playlist_is_protected(
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\"\n#EXTINF:6,\ns.ts\n"
        ));
        assert!(!media_playlist_is_protected("#EXTM3U\n#EXTINF:6,\ns.ts\n"));
        assert!(media_playlist_is_protected(
            "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\"\n#EXTINF:6,\ns.ts\n"
        ));
        assert!(media_playlist_is_protected("#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n#EXTINF:6,\ns.ts\n"));
    }

    #[test]
    fn skips_dash_content_protection() {
        let manifest =
            r#"<MPD><Period><AdaptationSet><ContentProtection /></AdaptationSet></Period></MPD>"#;
        assert!(parse_dash_formats("https://example.com/manifest.mpd", manifest).is_empty());
    }
}
