use ferric_common::{SubtitleFormat, SubtitleTrack};

/// Convert WebVTT text to basic SRT (Task 108).
pub fn vtt_to_srt(vtt: &str) -> String {
    let mut out = String::new();
    let mut index = 1u32;
    let mut lines = vtt.lines().peekable();
    // skip WEBVTT header
    while let Some(line) = lines.peek() {
        if line.contains("-->") {
            break;
        }
        lines.next();
    }
    while let Some(line) = lines.next() {
        if !line.contains("-->") {
            continue;
        }
        let timing = line
            .replace('.', ",")
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" ");
        // normalize "00:00:01,000 --> 00:00:02,000"
        let timing = if timing.matches(':').count() == 1 {
            // mm:ss.mmm → 00:mm:ss,mmm
            let parts: Vec<_> = timing.split(" --> ").collect();
            if parts.len() == 2 {
                format!(
                    "00:{}, --> 00:{}",
                    parts[0].replace('.', ","),
                    parts[1].replace('.', ",")
                )
                .replace(",,", ",")
            } else {
                timing
            }
        } else {
            timing
        };
        let mut text = String::new();
        for t in lines.by_ref() {
            if t.trim().is_empty() {
                break;
            }
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(t.trim());
        }
        if text.is_empty() {
            continue;
        }
        out.push_str(&format!("{index}\n{timing}\n{text}\n\n"));
        index += 1;
    }
    out
}

/// Filter subtitle tracks by preferred language codes (e.g. ["en","es"]).
pub fn filter_langs(tracks: &[SubtitleTrack], preferred: &[&str]) -> Vec<SubtitleTrack> {
    if preferred.is_empty() {
        return tracks.to_vec();
    }
    let mut chosen = Vec::new();
    for pref in preferred {
        for t in tracks {
            if t.lang.eq_ignore_ascii_case(pref)
                || t.lang
                    .to_ascii_lowercase()
                    .starts_with(&format!("{}-", pref.to_ascii_lowercase()))
            {
                if !chosen
                    .iter()
                    .any(|c: &SubtitleTrack| c.lang == t.lang && c.url == t.url)
                {
                    chosen.push(t.clone());
                }
            }
        }
    }
    if chosen.is_empty() {
        tracks.to_vec()
    } else {
        chosen
    }
}

/// Scan an HTML page for linked subtitle files (VTT or SRT).
/// Catches `<track>` elements and `<link>`/`<a>` tags pointing to subtitle files.
pub fn scan_html(html: &str) -> Vec<SubtitleTrack> {
    let mut tracks = Vec::new();

    // <track kind="subtitles" src="..." srclang="en" label="English">
    for chunk in html.split("<track ") {
        if !chunk.contains("src=") {
            continue;
        }
        let kind = attr_html(chunk, "kind").unwrap_or_default();
        if kind == "chapters" || kind == "metadata" {
            continue;
        }

        let src = match attr_html(chunk, "src") {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        let lang = attr_html(chunk, "srclang").unwrap_or_else(|| "und".to_string());
        let label = attr_html(chunk, "label");
        let fmt = subtitle_format_from_url(&src);

        tracks.push(SubtitleTrack {
            lang,
            name: label,
            url: src,
            format: fmt,
            is_auto_generated: false,
        });
    }

    // <a href="..."> or <link href="..."> pointing to .vtt / .srt
    for tag in ["<a ", "<link "] {
        for chunk in html.split(tag) {
            let href = match attr_html(chunk, "href") {
                Some(h) if looks_like_subtitle_url(&h) => h,
                _ => continue,
            };
            if tracks.iter().any(|t: &SubtitleTrack| t.url == href) {
                continue;
            }
            tracks.push(SubtitleTrack {
                lang: "und".to_string(),
                name: None,
                url: href,
                format: subtitle_format_from_url(
                    &tracks.last().map(|t| t.url.as_str()).unwrap_or(""),
                ),
                is_auto_generated: false,
            });
        }
    }

    tracks
}

fn looks_like_subtitle_url(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.ends_with(".vtt") || u.ends_with(".srt") || u.ends_with(".ttml") || u.ends_with(".dfxp")
}

fn subtitle_format_from_url(url: &str) -> SubtitleFormat {
    let u = url.to_ascii_lowercase();
    if u.ends_with(".vtt") {
        SubtitleFormat::Vtt
    } else if u.ends_with(".srt") {
        SubtitleFormat::Srt
    } else if u.ends_with(".ttml") || u.ends_with(".dfxp") {
        SubtitleFormat::Ttml
    } else {
        SubtitleFormat::Vtt
    }
}

/// Extract an XML attribute value from a tag line.
/// Handles both single- and double-quoted attribute values.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle_dq = format!(r#"{name}=""#);
    let needle_sq = format!("{name}='");
    if let Some(pos) = tag.find(&needle_dq) {
        let after = &tag[pos + needle_dq.len()..];
        Some(after.split('"').next().unwrap_or(""))
    } else if let Some(pos) = tag.find(&needle_sq) {
        let after = &tag[pos + needle_sq.len()..];
        Some(after.split('\'').next().unwrap_or(""))
    } else {
        None
    }
}

/// Same as `attr` but returns owned String (HTML attribute parsing).
fn attr_html(chunk: &str, name: &str) -> Option<String> {
    attr(chunk, name).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_html_track_elements() {
        let html = r#"<video><track kind="subtitles" src="/subs/en.vtt" srclang="en" label="English"><track kind="subtitles" src="/subs/fr.srt" srclang="fr" label="French"></video>"#;
        let tracks = scan_html(html);
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].lang, "en");
        assert!(matches!(tracks[0].format, SubtitleFormat::Vtt));
        assert_eq!(tracks[1].lang, "fr");
        assert!(matches!(tracks[1].format, SubtitleFormat::Srt));
    }

    #[test]
    fn scan_html_skips_chapters_tracks() {
        let html = r#"<track kind="chapters" src="/cues.vtt" srclang="en"><track kind="subtitles" src="/subs.vtt" srclang="de">"#;
        let tracks = scan_html(html);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].lang, "de");
    }

    #[test]
    fn vtt_converts_to_srt() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHello\n\n00:00:03.000 --> 00:00:04.000\nWorld\n";
        let srt = vtt_to_srt(vtt);
        assert!(srt.contains("1\n"));
        assert!(srt.contains("Hello"));
        assert!(srt.contains("World"));
        assert!(srt.contains("-->"));
    }

    #[test]
    fn filter_preferred_langs() {
        let tracks = vec![
            SubtitleTrack {
                lang: "en".into(),
                name: None,
                url: "u".into(),
                format: SubtitleFormat::Vtt,
                is_auto_generated: false,
            },
            SubtitleTrack {
                lang: "ja".into(),
                name: None,
                url: "j".into(),
                format: SubtitleFormat::Vtt,
                is_auto_generated: false,
            },
        ];
        let f = filter_langs(&tracks, &["ja"]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].lang, "ja");
    }
}
