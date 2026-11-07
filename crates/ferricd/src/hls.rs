//! HLS (M3U8) segment expansion + AES-128 decrypt helpers (Task 102).

use anyhow::{Context, Result};
use m3u8_rs::{KeyMethod, Playlist};
use url::Url;

#[derive(Debug, Clone)]
pub struct HlsSegment {
    pub url: String,
    /// AES-128 key URI + IV when present.
    pub key: Option<HlsKey>,
    /// Inclusive byte range inside `url` (`#EXT-X-BYTERANGE`); `None` = the whole resource.
    /// Many streams are one big file cut into ranges, so ignoring this downloads the file once per segment.
    pub range: Option<(u64, u64)>,
}

/// Always AES-128 — `parse_key` rejects every other EXT-X-KEY method.
#[derive(Debug, Clone)]
pub struct HlsKey {
    pub uri: String,
    pub iv: Option<[u8; 16]>,
}

pub async fn fetch_segments(manifest_url: &str, client: &reqwest::Client) -> Result<Vec<String>> {
    Ok(fetch_hls_segments(manifest_url, client)
        .await?
        .into_iter()
        .map(|s| s.url)
        .collect())
}

/// Best-variant segments with optional per-segment AES-128 key.
pub async fn fetch_hls_segments(
    manifest_url: &str,
    client: &reqwest::Client,
) -> Result<Vec<HlsSegment>> {
    fetch_hls_segments_inner(manifest_url, client, 0).await
}

async fn fetch_hls_segments_inner(
    manifest_url: &str,
    client: &reqwest::Client,
    depth: usize,
) -> Result<Vec<HlsSegment>> {
    if depth > 3 {
        return Err(anyhow::anyhow!("HLS playlist nesting is too deep"));
    }

    let bytes = client
        .get(manifest_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let playlist = m3u8_rs::parse_playlist_res(&bytes)
        .map_err(|err| anyhow::anyhow!("parse HLS manifest: {err:?}"))?;

    match playlist {
        Playlist::MediaPlaylist(media) => segments_from_media(manifest_url, &media),
        Playlist::MasterPlaylist(master) => {
            let variant = master
                .variants
                .iter()
                .max_by_key(|variant| variant.bandwidth)
                .ok_or_else(|| anyhow::anyhow!("HLS master playlist has no variants"))?;
            let variant_url = resolve_url(manifest_url, &variant.uri)?;
            Box::pin(fetch_hls_segments_inner(&variant_url, client, depth + 1)).await
        }
    }
}

/// The download list of one quality: the init segment (`#EXT-X-MAP`, needed first by fragmented MP4),
/// then every media segment with its byte range and AES-128 key.
fn segments_from_media(
    manifest_url: &str,
    media: &m3u8_rs::MediaPlaylist,
) -> Result<Vec<HlsSegment>> {
    let mut out = Vec::with_capacity(media.segments.len() + 1);
    let mut active_key: Option<HlsKey> = None;
    // An EXT-X-BYTERANGE without an offset continues where the previous range of the SAME file ended.
    let mut next_offset: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut current_map: Option<(String, Option<(u64, u64)>)> = None;
    for (idx, segment) in media.segments.iter().enumerate() {
        if let Some(key) = &segment.key {
            active_key = parse_key(manifest_url, key, idx as u64)?;
        }
        if let Some(map) = &segment.map {
            let url = resolve_url(manifest_url, &map.uri)?;
            let range = map
                .byte_range
                .as_ref()
                .filter(|r| r.length > 0)
                .map(|r| (r.offset.unwrap_or(0), r.offset.unwrap_or(0) + r.length - 1));
            let this = (url.clone(), range);
            if current_map.as_ref() != Some(&this) {
                out.push(HlsSegment {
                    url,
                    key: None,
                    range,
                });
                current_map = Some(this);
            }
        }
        let mut seg_key = active_key.clone();
        // Default IV = media sequence number when EXT-X-KEY has no IV.
        if let Some(ref mut k) = seg_key {
            if k.iv.is_none() {
                let seq = media.media_sequence + idx as u64;
                let mut iv = [0_u8; 16];
                iv[8..].copy_from_slice(&seq.to_be_bytes());
                k.iv = Some(iv);
            }
        }
        let url = resolve_url(manifest_url, &segment.uri)?;
        let range = segment
            .byte_range
            .as_ref()
            .filter(|r| r.length > 0)
            .map(|r| {
                let start = r
                    .offset
                    .unwrap_or_else(|| next_offset.get(&url).copied().unwrap_or(0));
                let end = start + r.length - 1;
                next_offset.insert(url.clone(), end + 1);
                (start, end)
            });
        out.push(HlsSegment {
            url,
            key: seg_key,
            range,
        });
    }
    Ok(out)
}

fn parse_key(
    manifest_url: &str,
    key: &m3u8_rs::Key,
    _segment_index: u64,
) -> Result<Option<HlsKey>> {
    match key.method {
        KeyMethod::None => Ok(None),
        KeyMethod::AES128 => {
            let uri = key
                .uri
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("EXT-X-KEY AES-128 missing URI"))?;
            let uri = resolve_url(manifest_url, uri)?;
            let iv = key.iv.as_deref().and_then(parse_iv);
            Ok(Some(HlsKey { uri, iv }))
        }
        _ => Err(anyhow::anyhow!(
            "unsupported HLS key method (only AES-128 / NONE)"
        )),
    }
}

fn parse_iv(raw: &str) -> Option<[u8; 16]> {
    let hex = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X"))?;
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0_u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Fetch AES-128 key bytes (16) from URI (http(s) or data:).
pub async fn fetch_aes_key(client: &reqwest::Client, uri: &str) -> Result<[u8; 16]> {
    if let Some(b64) = uri.strip_prefix("data:text/plain;base64,") {
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
            .context("decode data: AES key")?;
        let mut key = [0_u8; 16];
        if bytes.len() < 16 {
            return Err(anyhow::anyhow!("AES key too short"));
        }
        key.copy_from_slice(&bytes[..16]);
        return Ok(key);
    }
    crate::security::validate_url(uri).map_err(|e| anyhow::anyhow!("AES key URI rejected: {e}"))?;
    let bytes = client
        .get(uri)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    if bytes.len() < 16 {
        return Err(anyhow::anyhow!("AES key response too short"));
    }
    let mut key = [0_u8; 16];
    key.copy_from_slice(&bytes[..16]);
    Ok(key)
}

/// Decrypt AES-128 CBC PKCS7 media segment.
pub fn decrypt_aes128(ciphertext: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Result<Vec<u8>> {
    use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
    type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

    let mut buf = ciphertext.to_vec();
    let dec = Aes128CbcDec::new(key.into(), iv.into());
    let plain = dec
        .decrypt_padded::<Pkcs7>(&mut buf)
        .map_err(|e| anyhow::anyhow!("AES-128 decrypt failed: {e}"))?;
    Ok(plain.to_vec())
}

pub fn resolve_url(base: &str, value: &str) -> Result<String> {
    let base = Url::parse(base).with_context(|| format!("parse base URL {base}"))?;
    Ok(base
        .join(value)
        .with_context(|| format!("resolve segment URL {value}"))?
        .to_string())
}

/// Remux MPEG-TS / fMP4 segment concat → MP4 via ffmpeg when available.
pub async fn remux_to_mp4(input: &std::path::Path, output: &std::path::Path) -> Result<()> {
    let ffmpeg = crate::media::ffmpeg_path()
        .ok_or_else(|| anyhow::anyhow!("ffmpeg not on PATH for HLS remux"))?;
    let in_s = input.to_string_lossy().to_string();
    let out_s = output.to_string_lossy().to_string();
    // Prefer ADTS→ASC bitstream filter for AAC-in-TS; fall back to plain copy.
    for args in [
        vec![
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            in_s.as_str(),
            "-c",
            "copy",
            "-bsf:a",
            "aac_adtstoasc",
            out_s.as_str(),
        ],
        vec![
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            in_s.as_str(),
            "-c",
            "copy",
            out_s.as_str(),
        ],
    ] {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let work = input.parent();
        match crate::ffmpeg_sandbox::run_sandboxed(&ffmpeg, &owned, work).await {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(_) => {}
            Err(e) => tracing::debug!(error = %e, "sandboxed ffmpeg remux attempt failed"),
        }
    }
    anyhow::bail!("ffmpeg HLS remux failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_hls_url_against_manifest_directory() -> Result<()> {
        assert_eq!(
            resolve_url("https://example.com/video/master/playlist.m3u8", "seg-1.ts")?,
            "https://example.com/video/master/seg-1.ts"
        );
        assert_eq!(
            resolve_url(
                "https://example.com/video/master/playlist.m3u8",
                "../seg-2.ts"
            )?,
            "https://example.com/video/seg-2.ts"
        );
        Ok(())
    }

    fn media(text: &str) -> m3u8_rs::MediaPlaylist {
        m3u8_rs::parse_media_playlist_res(text.as_bytes()).expect("test playlist parses")
    }

    #[test]
    fn byte_ranges_continue_per_file_and_the_init_segment_comes_first() -> Result<()> {
        let list = segments_from_media(
            "https://cdn.test/v/index.m3u8",
            &media(
                "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:6\n#EXT-X-MAP:URI=\"main.mp4\",BYTERANGE=\"719@0\"\n\
                 #EXTINF:6,\n#EXT-X-BYTERANGE:1000@719\nmain.mp4\n\
                 #EXTINF:6,\n#EXT-X-BYTERANGE:500\nmain.mp4\n\
                 #EXTINF:6,\n#EXT-X-BYTERANGE:250\nother.mp4\n#EXT-X-ENDLIST\n",
            ),
        )?;
        let got: Vec<(&str, Option<(u64, u64)>)> =
            list.iter().map(|s| (s.url.as_str(), s.range)).collect();
        assert_eq!(
            got,
            vec![
                ("https://cdn.test/v/main.mp4", Some((0, 718))), // init segment
                ("https://cdn.test/v/main.mp4", Some((719, 1718))), // explicit offset
                ("https://cdn.test/v/main.mp4", Some((1719, 2218))), // continues after the previous range
                ("https://cdn.test/v/other.mp4", Some((0, 249))),    // a different file starts at 0
            ]
        );
        Ok(())
    }

    #[test]
    fn plain_segments_have_no_range_and_a_repeated_map_is_fetched_once() -> Result<()> {
        let list = segments_from_media(
            "https://cdn.test/v/index.m3u8",
            &media("#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6,\na.m4s\n#EXTINF:6,\nb.m4s\n#EXT-X-ENDLIST\n"),
        )?;
        assert_eq!(list.len(), 3, "init + 2 segments");
        assert!(list.iter().all(|s| s.range.is_none()));
        assert_eq!(list[0].url, "https://cdn.test/v/init.mp4");
        Ok(())
    }

    #[test]
    fn parse_iv_hex() {
        let iv = parse_iv("0x0000000000000000000000000000000A").unwrap();
        assert_eq!(iv[15], 0x0A);
    }

    #[test]
    fn decrypt_roundtrip_pkcs7() {
        use aes::cipher::{block_padding::Pkcs7, BlockModeEncrypt, KeyIvInit};
        type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
        let key = [0x42_u8; 16];
        let iv = [0x11_u8; 16];
        let plain = b"hello HLS segment!!"; // 19 bytes
        let mut buf = plain.to_vec();
        // pad buffer
        buf.resize(32, 0);
        let enc = Aes128CbcEnc::new(&key.into(), &iv.into());
        let ct_len = enc
            .encrypt_padded::<Pkcs7>(&mut buf, plain.len())
            .unwrap()
            .len();
        let ct = &buf[..ct_len];
        let out = decrypt_aes128(ct, &key, &iv).unwrap();
        assert_eq!(out, plain);
    }
}
