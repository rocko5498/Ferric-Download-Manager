use anyhow::{bail, Result};
use ferric_common::ExtractedFormat;

#[derive(Debug, Clone, PartialEq)]
pub enum FormatSelection {
    Single(ExtractedFormat),
    Merge {
        video: ExtractedFormat,
        audio: ExtractedFormat,
    },
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Select formats from `formats` using a yt-dlp-style spec string.
///
/// Grammar (subset):
///   spec   = term ('/' term)*
///   term   = stream ('+' stream)?
///   stream = selector filter*
///   selector = 'best'|'worst'|'bestvideo'|'bestaudio'|'worstvideo'|'worstaudio'
///   filter = '[' key op value ']'
///   op     = '='|'!='|'<='|'>='|'<'|'>'
///
/// Examples:
///   "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[height<=1080]/best"
pub fn select(formats: &[ExtractedFormat], spec: &str) -> Result<FormatSelection> {
    for term_str in spec.split('/') {
        let term_str = term_str.trim();
        if let Ok(sel) = eval_term(formats, term_str) {
            return Ok(sel);
        }
    }
    bail!("no format matched spec: {spec}")
}

// ── Term evaluation ──────────────────────────────────────────────────────────

fn eval_term(formats: &[ExtractedFormat], term: &str) -> Result<FormatSelection> {
    if let Some((left, right)) = split_merge(term) {
        let video = pick_stream(formats, left.trim(), StreamKind::Video)?;
        let audio = pick_stream(formats, right.trim(), StreamKind::Audio)?;
        return Ok(FormatSelection::Merge { video, audio });
    }
    Ok(FormatSelection::Single(pick_stream(
        formats,
        term,
        StreamKind::Any,
    )?))
}

/// Split "a+b" at the `+` not inside brackets.
fn split_merge(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            '+' if depth == 0 => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}

#[derive(Clone, Copy)]
enum StreamKind {
    Video,
    Audio,
    Any,
}

fn pick_stream(
    formats: &[ExtractedFormat],
    stream_expr: &str,
    kind: StreamKind,
) -> Result<ExtractedFormat> {
    let (selector, filters) = parse_stream(stream_expr)?;

    let mut candidates: Vec<&ExtractedFormat> = formats
        .iter()
        .filter(|f| !f.has_drm)
        .filter(|f| match kind {
            StreamKind::Video => !f.is_audio_only,
            StreamKind::Audio => f.is_audio_only,
            StreamKind::Any => true,
        })
        .filter(|f| apply_filters(f, &filters))
        .collect();

    if candidates.is_empty() {
        bail!("no formats matched stream expr: {stream_expr}");
    }

    let picked = match selector {
        Sel::Best | Sel::BestVideo | Sel::BestAudio => {
            candidates.sort_by(|a, b| {
                score(b)
                    .partial_cmp(&score(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            candidates[0]
        }
        Sel::Worst | Sel::WorstVideo | Sel::WorstAudio => {
            candidates.sort_by(|a, b| {
                score(a)
                    .partial_cmp(&score(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            candidates[0]
        }
    };
    Ok(picked.clone())
}

fn score(f: &ExtractedFormat) -> f64 {
    f64::from(f.height.unwrap_or(0)) * 1000.0
        + f.tbr_kbps.unwrap_or(0.0)
        + f.abr_kbps.unwrap_or(0.0)
}

// ── Parser ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum Sel {
    Best,
    Worst,
    BestVideo,
    BestAudio,
    WorstVideo,
    WorstAudio,
}

#[derive(Debug, Clone)]
struct Filter {
    key: String,
    op: Op,
    value: FVal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Eq,
    Ne,
    Le,
    Ge,
    Lt,
    Gt,
}

#[derive(Debug, Clone)]
enum FVal {
    Str(String),
    Num(f64),
}

fn parse_stream(s: &str) -> Result<(Sel, Vec<Filter>)> {
    let (sel_str, rest) = match s.find('[') {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    };
    let sel = match sel_str.trim() {
        "best" => Sel::Best,
        "worst" => Sel::Worst,
        "bestvideo" => Sel::BestVideo,
        "bestaudio" => Sel::BestAudio,
        "worstvideo" => Sel::WorstVideo,
        "worstaudio" => Sel::WorstAudio,
        other => bail!("unknown selector: {other}"),
    };
    let filters = parse_filters(rest)?;
    Ok((sel, filters))
}

fn parse_filters(mut s: &str) -> Result<Vec<Filter>> {
    let mut out = Vec::new();
    while let Some(start) = s.find('[') {
        let end = s[start..]
            .find(']')
            .map(|i| start + i)
            .ok_or_else(|| anyhow::anyhow!("unclosed ["))?;
        out.push(parse_filter(&s[start + 1..end])?);
        s = &s[end + 1..];
    }
    Ok(out)
}

fn parse_filter(inner: &str) -> Result<Filter> {
    for op_str in &["!=", "<=", ">=", "<", ">", "="] {
        if let Some(pos) = inner.find(op_str) {
            let key = inner[..pos].trim().to_string();
            let val_str = inner[pos + op_str.len()..].trim();
            let op = match *op_str {
                "=" => Op::Eq,
                "!=" => Op::Ne,
                "<=" => Op::Le,
                ">=" => Op::Ge,
                "<" => Op::Lt,
                ">" => Op::Gt,
                _ => unreachable!(),
            };
            let value = if let Ok(n) = val_str.parse::<f64>() {
                FVal::Num(n)
            } else {
                FVal::Str(val_str.to_string())
            };
            return Ok(Filter { key, op, value });
        }
    }
    bail!("cannot parse filter: [{inner}]")
}

fn apply_filters(f: &ExtractedFormat, filters: &[Filter]) -> bool {
    filters.iter().all(|filter| apply_filter(f, filter))
}

fn apply_filter(f: &ExtractedFormat, filter: &Filter) -> bool {
    match filter.key.as_str() {
        "ext" => cmp_str(f.ext.as_str(), filter.op, &filter.value),
        "vcodec" => cmp_opt_str(f.vcodec.as_deref(), filter.op, &filter.value),
        "acodec" => cmp_opt_str(f.acodec.as_deref(), filter.op, &filter.value),
        "protocol" => cmp_opt_str(f.protocol.as_deref(), filter.op, &filter.value),
        "height" => cmp_num(f.height.map(f64::from), filter.op, &filter.value),
        "width" => cmp_num(f.width.map(f64::from), filter.op, &filter.value),
        "tbr" => cmp_num(f.tbr_kbps, filter.op, &filter.value),
        "abr" => cmp_num(f.abr_kbps, filter.op, &filter.value),
        "fps" => cmp_num(f.fps, filter.op, &filter.value),
        "filesize" => cmp_num(f.filesize.map(|s| s as f64), filter.op, &filter.value),
        _ => false,
    }
}

fn cmp_str(actual: &str, op: Op, val: &FVal) -> bool {
    let FVal::Str(s) = val else { return false };
    match op {
        Op::Eq => actual == s,
        Op::Ne => actual != s,
        _ => false,
    }
}

fn cmp_opt_str(actual: Option<&str>, op: Op, val: &FVal) -> bool {
    actual.map_or(false, |a| cmp_str(a, op, val))
}

fn cmp_num(actual: Option<f64>, op: Op, val: &FVal) -> bool {
    let (Some(a), FVal::Num(v)) = (actual, val) else {
        return false;
    };
    match op {
        Op::Eq => (a - v).abs() < f64::EPSILON,
        Op::Ne => (a - v).abs() >= f64::EPSILON,
        Op::Le => a <= *v,
        Op::Ge => a >= *v,
        Op::Lt => a < *v,
        Op::Gt => a > *v,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ferric_common::ExtractedFormat;

    fn fmt(id: &str, height: u32, ext: &str, audio_only: bool, tbr: f64) -> ExtractedFormat {
        ExtractedFormat {
            id: id.to_string(),
            label: id.to_string(),
            url: format!("https://example.com/{id}"),
            ext: ext.to_string(),
            protocol: Some("https".to_string()),
            manifest_url: None,
            format_note: None,
            width: None,
            height: Some(height),
            tbr_kbps: Some(tbr),
            abr_kbps: None,
            vbr_kbps: None,
            fps: None,
            vcodec: Some("avc1".to_string()),
            acodec: if audio_only {
                Some("mp4a".to_string())
            } else {
                None
            },
            container: None,
            language: None,
            filesize: None,
            has_drm: false,
            headers: vec![],
            is_audio_only: audio_only,
        }
    }

    #[test]
    fn best_picks_highest_height() {
        let formats = vec![
            fmt("sd", 480, "mp4", false, 800.0),
            fmt("hd", 1080, "mp4", false, 4000.0),
        ];
        let sel = select(&formats, "best").unwrap();
        assert!(matches!(sel, FormatSelection::Single(f) if f.id == "hd"));
    }

    #[test]
    fn filter_by_height() {
        let formats = vec![
            fmt("sd", 480, "mp4", false, 800.0),
            fmt("hd", 1080, "mp4", false, 4000.0),
        ];
        let sel = select(&formats, "best[height<=720]").unwrap();
        assert!(matches!(sel, FormatSelection::Single(f) if f.id == "sd"));
    }

    #[test]
    fn filter_by_ext() {
        let formats = vec![
            fmt("mp4", 1080, "mp4", false, 4000.0),
            fmt("webm", 1080, "webm", false, 3000.0),
        ];
        let sel = select(&formats, "best[ext=webm]").unwrap();
        assert!(matches!(sel, FormatSelection::Single(f) if f.id == "webm"));
    }

    #[test]
    fn merge_term() {
        let formats = vec![
            fmt("vid", 1080, "mp4", false, 4000.0),
            fmt("aud", 0, "m4a", true, 128.0),
        ];
        let sel = select(&formats, "bestvideo+bestaudio").unwrap();
        assert!(matches!(sel, FormatSelection::Merge { .. }));
    }

    #[test]
    fn fallback_slash() {
        let formats = vec![fmt("sd", 480, "mp4", false, 800.0)];
        // First term needs 1080 (no match), fallback to best
        let sel = select(&formats, "best[height>=1080]/best").unwrap();
        assert!(matches!(sel, FormatSelection::Single(f) if f.id == "sd"));
    }

    #[test]
    fn drm_excluded() {
        let mut f = fmt("drm", 1080, "mp4", false, 9000.0);
        f.has_drm = true;
        let formats = vec![f, fmt("clean", 720, "mp4", false, 3000.0)];
        let sel = select(&formats, "best").unwrap();
        assert!(matches!(sel, FormatSelection::Single(f) if f.id == "clean"));
    }

    #[test]
    fn no_match_returns_err() {
        let formats = vec![fmt("sd", 480, "mp4", false, 800.0)];
        assert!(select(&formats, "best[ext=mkv]").is_err());
    }
}
