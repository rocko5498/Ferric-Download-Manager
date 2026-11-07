pub mod bandcamp;
pub mod bilibili;
pub mod cookies;
pub mod dailymotion;
pub mod engine;
pub mod facebook;
pub mod generic;
pub mod imgur;
pub mod info_json;
pub mod instagram;
pub mod manifest;
pub mod odysee;
pub mod pinterest;
pub mod playlist;
pub mod reddit;
pub mod rumble;
pub mod selector;
pub mod soundcloud;
pub mod subtitles;
pub mod tiktok;
pub mod twitch;
pub mod twitter;
pub mod util;
pub mod vimeo;
pub mod vk;
pub mod youtube;
// Task 104
pub mod crunchyroll;
pub mod espn;
pub mod hotstar;
pub mod kick;
pub mod niconico;
// Task 105
pub mod arte;
pub mod bbc_iplayer;
pub mod nhk;
pub mod ted;
pub mod zdf;
// Pure-Rust port expansion
pub mod archive;
pub mod bitchute;
pub mod okru;
pub mod peertube;
pub mod streamable;
// Pure-Rust IE expansion (continue port)
pub mod gofile;
pub mod loom;
pub mod mediafire;
pub mod mixcloud;
pub mod newgrounds;
pub mod pixeldrain;
// Task 111 / 115 helpers
pub mod netscape;
pub mod presets;
// Optional standalone yt-dlp *binary* bridge (not Python; not the product plan)
#[cfg(feature = "ytdlp-subprocess")]
pub mod ytdlp;

use anyhow::{Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};

pub struct Extractor {
    pub name: &'static str,
    pub matches: fn(&str) -> bool,
}

pub const REGISTRY: &[Extractor] = &[
    Extractor {
        name: "youtube",
        matches: youtube::matches,
    },
    Extractor {
        name: "vimeo",
        matches: vimeo::matches,
    },
    Extractor {
        name: "reddit",
        matches: reddit::matches,
    },
    Extractor {
        name: "twitter",
        matches: twitter::matches,
    },
    Extractor {
        name: "tiktok",
        matches: tiktok::matches,
    },
    Extractor {
        name: "soundcloud",
        matches: soundcloud::matches,
    },
    Extractor {
        name: "twitch",
        matches: twitch::matches,
    },
    Extractor {
        name: "dailymotion",
        matches: dailymotion::matches,
    },
    Extractor {
        name: "bandcamp",
        matches: bandcamp::matches,
    },
    Extractor {
        name: "bilibili",
        matches: bilibili::matches,
    },
    Extractor {
        name: "instagram",
        matches: instagram::matches,
    },
    Extractor {
        name: "facebook",
        matches: facebook::matches,
    },
    Extractor {
        name: "pinterest",
        matches: pinterest::matches,
    },
    Extractor {
        name: "imgur",
        matches: imgur::matches,
    },
    Extractor {
        name: "vk",
        matches: vk::matches,
    },
    Extractor {
        name: "rumble",
        matches: rumble::matches,
    },
    Extractor {
        name: "odysee",
        matches: odysee::matches,
    },
    Extractor {
        name: "kick",
        matches: kick::matches,
    },
    Extractor {
        name: "espn",
        matches: espn::matches,
    },
    Extractor {
        name: "niconico",
        matches: niconico::matches,
    },
    Extractor {
        name: "crunchyroll",
        matches: crunchyroll::matches,
    },
    Extractor {
        name: "hotstar",
        matches: hotstar::matches,
    },
    Extractor {
        name: "bbc_iplayer",
        matches: bbc_iplayer::matches,
    },
    Extractor {
        name: "arte",
        matches: arte::matches,
    },
    Extractor {
        name: "zdf",
        matches: zdf::matches,
    },
    Extractor {
        name: "nhk",
        matches: nhk::matches,
    },
    Extractor {
        name: "ted",
        matches: ted::matches,
    },
    Extractor {
        name: "archive",
        matches: archive::matches,
    },
    Extractor {
        name: "streamable",
        matches: streamable::matches,
    },
    Extractor {
        name: "bitchute",
        matches: bitchute::matches,
    },
    Extractor {
        name: "peertube",
        matches: peertube::matches,
    },
    Extractor {
        name: "okru",
        matches: okru::matches,
    },
    Extractor {
        name: "loom",
        matches: loom::matches,
    },
    Extractor {
        name: "mixcloud",
        matches: mixcloud::matches,
    },
    Extractor {
        name: "newgrounds",
        matches: newgrounds::matches,
    },
    Extractor {
        name: "mediafire",
        matches: mediafire::matches,
    },
    Extractor {
        name: "pixeldrain",
        matches: pixeldrain::matches,
    },
    Extractor {
        name: "gofile",
        matches: gofile::matches,
    },
];

/// Product extract configuration.
///
/// Default = **pure Rust only** (product plan). Optional standalone yt-dlp binary
/// is opt-in via `force_ytdlp` or `pure_rust_only = false` (settings-backed).
#[derive(Debug, Clone)]
pub struct ExtractConfig {
    /// Skip optional yt-dlp binary entirely. **Default true** — pure Rust plan.
    pub pure_rust_only: bool,
    /// Always use yt-dlp binary (opt-in max compatibility). Skips native IEs.
    pub force_ytdlp: bool,
    /// Absolute path to yt-dlp binary (settings / packaging). Empty = PATH / FERRIC_YTDLP.
    pub ytdlp_binary: Option<std::path::PathBuf>,
    /// Absolute Netscape cookies.txt for login-walled sites.
    pub cookies_file: Option<std::path::PathBuf>,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        Self {
            pure_rust_only: true,
            force_ytdlp: false,
            ytdlp_binary: None,
            cookies_file: None,
        }
    }
}

/// Product extract path — **pure Rust first**.
///
/// 1. Native IE / generic (Rust).
/// 2. Optional yt-dlp **binary** only if `!pure_rust_only` or `force_ytdlp` (settings).
/// 3. Download remains Ferric (ferricd) — never Python.
#[tracing::instrument(skip(client))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    extract_with_config(url, client, ExtractConfig::default()).await
}

/// Full product extract with settings-backed yt-dlp options.
#[tracing::instrument(skip(client, cfg))]
pub async fn extract_with_config(
    url: &str,
    client: &reqwest::Client,
    cfg: ExtractConfig,
) -> Result<ExtractResult> {
    // YouTube requires its fast-moving official extractor. The former embedded
    // JavaScript engine carried an unpatched memory-safety advisory.
    if youtube::matches(url) {
        return extract_via_ytdlp(
            url,
            client,
            cfg.cookies_file.clone(),
            cfg.ytdlp_binary.clone(),
        )
        .await;
    }

    if cfg.force_ytdlp && !cfg.pure_rust_only {
        return extract_via_ytdlp(
            url,
            client,
            cfg.cookies_file.clone(),
            cfg.ytdlp_binary.clone(),
        )
        .await;
    }

    match extract_native(url, client).await {
        Ok(result) if extraction_usable(&result) => Ok(result),
        Ok(thin) if cfg.pure_rust_only => {
            if extraction_usable(&thin) {
                Ok(thin)
            } else {
                Err(anyhow::anyhow!(
                    "native extractor '{}' returned no formats (pure-Rust-only mode)",
                    thin.extractor
                ))
            }
        }
        Ok(thin) => try_ytdlp_engine(url, client, Some(thin), &cfg).await,
        Err(native_err) if cfg.pure_rust_only => Err(native_err)
            .context("native extraction failed (pure-Rust-only mode; yt-dlp engine disabled)"),
        Err(native_err) => match try_ytdlp_engine(url, client, None, &cfg).await {
            Ok(y) => Ok(y),
            Err(yerr) => Err(native_err).context(format!("yt-dlp engine also failed: {yerr}")),
        },
    }
}

/// Force the yt-dlp engine (skip native IEs). Product “max compatibility” mode.
#[cfg(feature = "ytdlp-subprocess")]
#[tracing::instrument(skip(client))]
pub async fn extract_via_ytdlp(
    url: &str,
    client: &reqwest::Client,
    cookies_file: Option<std::path::PathBuf>,
    binary: Option<std::path::PathBuf>,
) -> Result<ExtractResult> {
    let opts = ytdlp::YtdlpOptions {
        cookies_file,
        binary,
        timeout: None,
    };
    let mut y = ytdlp::extract(url, opts).await?;
    manifest::expand_manifest_formats(client, &mut y.formats).await;
    sort_formats(&mut y.formats);
    Ok(y)
}

#[cfg(not(feature = "ytdlp-subprocess"))]
pub async fn extract_via_ytdlp(
    _url: &str,
    _client: &reqwest::Client,
    _cookies_file: Option<std::path::PathBuf>,
    _binary: Option<std::path::PathBuf>,
) -> Result<ExtractResult> {
    Err(anyhow::anyhow!(
        "yt-dlp engine not built in (enable feature ytdlp-subprocess)"
    ))
}

#[cfg(feature = "ytdlp-subprocess")]
async fn try_ytdlp_engine(
    url: &str,
    client: &reqwest::Client,
    thin: Option<ExtractResult>,
    cfg: &ExtractConfig,
) -> Result<ExtractResult> {
    tracing::info!("using yt-dlp engine for site coverage");
    let opts = ytdlp::YtdlpOptions {
        cookies_file: cfg.cookies_file.clone(),
        binary: cfg.ytdlp_binary.clone(),
        timeout: None,
    };
    match ytdlp::extract(url, opts).await {
        Ok(mut y) => {
            manifest::expand_manifest_formats(client, &mut y.formats).await;
            sort_formats(&mut y.formats);
            Ok(y)
        }
        Err(yerr) => {
            if let Some(thin) = thin {
                if !thin.formats.is_empty() || !thin.playlist_entries.is_empty() {
                    return Ok(thin);
                }
            }
            Err(yerr).context(
                "yt-dlp engine failed — install from https://github.com/yt-dlp/yt-dlp, \
                 set Settings → Media → yt-dlp path, or set FERRIC_YTDLP",
            )
        }
    }
}

#[cfg(not(feature = "ytdlp-subprocess"))]
async fn try_ytdlp_engine(
    url: &str,
    _client: &reqwest::Client,
    thin: Option<ExtractResult>,
    _cfg: &ExtractConfig,
) -> Result<ExtractResult> {
    let _ = url;
    if let Some(thin) = thin {
        if !thin.formats.is_empty() || !thin.playlist_entries.is_empty() {
            return Ok(thin);
        }
        return Err(anyhow::anyhow!(
            "native extractor '{}' unusable and yt-dlp engine not compiled in",
            thin.extractor
        ));
    }
    Err(anyhow::anyhow!(
        "no native IE matched and yt-dlp engine not compiled in (feature ytdlp-subprocess)"
    ))
}

/// Native-only extraction (no yt-dlp). Useful for tests and diagnostics.
#[tracing::instrument(skip(client))]
pub async fn extract_native(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    if playlist::matches(url) {
        tracing::debug!("dispatching to playlist expander");
        return playlist::expand(url, client).await;
    }

    let name = extractor_name(url);
    tracing::debug!(?name, "dispatching extractor");

    let mut result = match dispatch_site_ie(name, url, client).await {
        Ok(r) => r,
        Err(_) if name.is_none() => generic::extract(url, client).await?,
        Err(e) => return Err(e),
    };

    manifest::expand_manifest_formats(client, &mut result.formats).await;
    sort_formats(&mut result.formats);
    Ok(result)
}

/// Dispatch a known REGISTRY site IE only (never generic — avoids async recursion).
pub async fn dispatch_site_ie(
    name: Option<&str>,
    url: &str,
    client: &reqwest::Client,
) -> Result<ExtractResult> {
    match name {
        Some("youtube") => youtube::extract(url, client).await,
        Some("vimeo") => vimeo::extract(url, client).await,
        Some("reddit") => reddit::extract(url, client).await,
        Some("twitter") => twitter::extract(url, client).await,
        Some("tiktok") => tiktok::extract(url, client).await,
        Some("soundcloud") => soundcloud::extract(url, client).await,
        Some("twitch") => twitch::extract(url, client).await,
        Some("dailymotion") => dailymotion::extract(url, client).await,
        Some("bandcamp") => bandcamp::extract(url, client).await,
        Some("bilibili") => bilibili::extract(url, client).await,
        Some("instagram") => instagram::extract(url, client).await,
        Some("facebook") => facebook::extract(url, client).await,
        Some("pinterest") => pinterest::extract(url, client).await,
        Some("imgur") => imgur::extract(url, client).await,
        Some("vk") => vk::extract(url, client).await,
        Some("rumble") => rumble::extract(url, client).await,
        Some("odysee") => odysee::extract(url, client).await,
        Some("kick") => kick::extract(url, client).await,
        Some("espn") => espn::extract(url, client).await,
        Some("niconico") => niconico::extract(url, client).await,
        Some("crunchyroll") => crunchyroll::extract(url, client).await,
        Some("hotstar") => hotstar::extract(url, client).await,
        Some("bbc_iplayer") => bbc_iplayer::extract(url, client).await,
        Some("arte") => arte::extract(url, client).await,
        Some("zdf") => zdf::extract(url, client).await,
        Some("nhk") => nhk::extract(url, client).await,
        Some("ted") => ted::extract(url, client).await,
        Some("archive") => archive::extract(url, client).await,
        Some("streamable") => streamable::extract(url, client).await,
        Some("bitchute") => bitchute::extract(url, client).await,
        Some("peertube") => peertube::extract(url, client).await,
        Some("okru") => okru::extract(url, client).await,
        Some("loom") => loom::extract(url, client).await,
        Some("mixcloud") => mixcloud::extract(url, client).await,
        Some("newgrounds") => newgrounds::extract(url, client).await,
        Some("mediafire") => mediafire::extract(url, client).await,
        Some("pixeldrain") => pixeldrain::extract(url, client).await,
        Some("gofile") => gofile::extract(url, client).await,
        Some(other) => Err(anyhow::anyhow!("no site IE dispatcher for '{other}'")),
        None => Err(anyhow::anyhow!("no REGISTRY match")),
    }
}

fn extraction_usable(result: &ExtractResult) -> bool {
    !result.formats.is_empty() || !result.playlist_entries.is_empty()
}

pub fn is_known_site(url: &str) -> bool {
    extractor_name(url).is_some() || playlist::matches(url)
}

pub fn extractor_name(url: &str) -> Option<&'static str> {
    REGISTRY
        .iter()
        .find(|extractor| (extractor.matches)(url))
        .map(|extractor| extractor.name)
}

pub fn sort_formats(formats: &mut [ExtractedFormat]) {
    formats.sort_by(|a, b| {
        format_score(b)
            .partial_cmp(&format_score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn format_score(format: &ExtractedFormat) -> f64 {
    if format.has_drm {
        return -10_000.0;
    }
    let height = f64::from(format.height.unwrap_or(0));
    let bitrate = format.tbr_kbps.unwrap_or(0.0);
    let combined_bonus = if format.is_audio_only { 0.0 } else { 1_000.0 };
    let manifest_bonus = match format.protocol.as_deref() {
        Some("m3u8" | "dash") => 200.0,
        _ => 0.0,
    };
    combined_bonus + manifest_bonus + height + bitrate / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(
        id: &str,
        height: Option<u32>,
        tbr_kbps: Option<f64>,
        protocol: Option<&str>,
    ) -> ExtractedFormat {
        ExtractedFormat {
            id: id.to_string(),
            label: id.to_string(),
            url: format!("https://example.com/{id}.mp4"),
            ext: "mp4".to_string(),
            protocol: protocol.map(str::to_string),
            manifest_url: None,
            format_note: None,
            width: None,
            height,
            tbr_kbps,
            abr_kbps: None,
            vbr_kbps: None,
            fps: None,
            vcodec: None,
            acodec: None,
            container: Some("mp4".to_string()),
            language: None,
            filesize: None,
            has_drm: false,
            headers: Vec::new(),
            is_audio_only: false,
        }
    }

    #[test]
    fn registry_identifies_known_sites() {
        assert_eq!(extractor_name("https://youtu.be/abc"), Some("youtube"));
        assert_eq!(extractor_name("https://vimeo.com/123"), Some("vimeo"));
        assert_eq!(
            extractor_name("https://www.reddit.com/r/videos/comments/abc/title/"),
            Some("reddit")
        );
        assert_eq!(
            extractor_name("https://www.instagram.com/p/CxYZ/"),
            Some("instagram")
        );
        assert_eq!(
            extractor_name("https://www.facebook.com/watch/?v=123"),
            Some("facebook")
        );
        assert_eq!(
            extractor_name("https://pinterest.com/pin/123/"),
            Some("pinterest")
        );
        assert_eq!(
            extractor_name("https://imgur.com/gallery/123"),
            Some("imgur")
        );
        assert_eq!(extractor_name("https://vk.com/video-123_456"), Some("vk"));
        assert_eq!(extractor_name("https://kick.com/xqc"), Some("kick"));
        assert_eq!(extractor_name("https://www.ted.com/talks/x"), Some("ted"));
        assert_eq!(
            extractor_name("https://www.hotstar.com/in/x"),
            Some("hotstar")
        );
        assert_eq!(
            extractor_name("https://www.arte.tv/en/videos/1/"),
            Some("arte")
        );
        assert_eq!(
            extractor_name("https://archive.org/details/foo"),
            Some("archive")
        );
        assert_eq!(
            extractor_name("https://streamable.com/abc"),
            Some("streamable")
        );
        assert_eq!(extractor_name("https://ok.ru/video/1"), Some("okru"));
        assert_eq!(
            extractor_name("https://rumble.com/v5pv5f-video.html"),
            Some("rumble")
        );
        assert_eq!(
            extractor_name("https://odysee.com/@creator/video"),
            Some("odysee")
        );
        assert_eq!(extractor_name("https://example.com/video.mp4"), None);
    }

    #[test]
    fn playlist_is_known_site() {
        assert!(is_known_site(
            "https://www.youtube.com/playlist?list=PLabcdef"
        ));
    }

    #[test]
    fn sort_formats_prefers_best_quality() {
        let mut formats = vec![
            format("480p", Some(480), Some(1200.0), Some("https")),
            format("1080p", Some(1080), Some(4200.0), Some("https")),
        ];

        sort_formats(&mut formats);

        assert_eq!(formats[0].id, "1080p");
    }
}
