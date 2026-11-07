use anyhow::Result;
use std::collections::{HashSet, VecDeque};

/// Fetch `url`, extract all hyperlinks from the HTML response, and return the
/// absolute URLs filtered (optionally) by file extension.
///
/// `depth = 0` → single page (original behaviour).
/// `depth = 1..=5` → BFS crawl up to that many levels, same origin only.
/// IDM's Site Grabber page filters (project wizard step 2): `*` patterns, case-insensitive.
#[derive(Default)]
pub struct GrabFilters {
    /// Never open or collect these (logout pages, "don't explore the following paths/domains").
    pub exclude: Vec<String>,
    /// When not empty, only pages matching one of these are explored ("explore ... within the following paths only").
    pub include: Vec<String>,
    /// "Don't explore parent directories": only pages under the start page's directory are explored.
    pub stay_below: bool,
    /// IDM's "levels on other sites": how many link levels to follow once the crawl has left the start site.
    pub external_levels: u8,
}

fn matches_any(patterns: &[String], url: &str) -> bool {
    let url = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_ascii_lowercase();
    patterns.iter().any(|p| {
        let p = p
            .trim()
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .to_ascii_lowercase();
        !p.is_empty() && crate::site_logins::glob(&p, &url)
    })
}

pub async fn grab_links(
    url: &str,
    extensions: &[String],
    depth: u8,
    filters: &GrabFilters,
) -> Result<Vec<String>> {
    crate::security::validate_url(url)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    let seed = reqwest::Url::parse(url)?;
    // Full origin: another port is another site.
    let origin = seed.origin().ascii_serialization();
    let base_dir = {
        let path = seed.path();
        let dir = path
            .rsplit_once('/')
            .map_or("/", |(d, _)| if d.is_empty() { "/" } else { d });
        format!("{origin}{}", dir.trim_end_matches('/'))
    };

    let mut visited: HashSet<String> = HashSet::new();
    let mut collected: Vec<String> = Vec::new();

    // BFS queue: (url_string, remaining_depth, levels left on other sites)
    let mut queue: VecDeque<(String, u8, u8)> = VecDeque::new();
    queue.push_back((url.to_string(), depth, filters.external_levels.min(5)));
    visited.insert(url.to_string());

    while let Some((current_url, remaining, ext_left)) = queue.pop_front() {
        if crate::security::validate_url(&current_url).is_err() {
            tracing::debug!("grab: skip invalid/ssrf URL {current_url}");
            continue;
        }
        let response = match client.get(&current_url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                // Non-2xx: skip silently (might be a binary/download link)
                tracing::debug!("grab: {current_url} returned {}", r.status());
                continue;
            }
            Err(e) => {
                tracing::debug!("grab: {current_url} error: {e}");
                continue;
            }
        };

        let base = response.url().clone();

        // Only parse HTML pages; binary downloads shouldn't be crawled
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_html = content_type.contains("html");

        if !is_html {
            // It's a direct download link — collect it
            collected.push(current_url.clone());
            continue;
        }

        let html = match response.text().await {
            Ok(t) => t,
            Err(_) => continue,
        };

        let links = extract_links(&html, &base);

        for link in links {
            if visited.contains(&link) {
                continue;
            }
            visited.insert(link.clone());
            if crate::security::validate_url(&link).is_err() || matches_any(&filters.exclude, &link)
            {
                continue;
            }

            let same_origin = reqwest::Url::parse(&link)
                .is_ok_and(|u| u.origin().ascii_serialization() == origin);
            let may_explore = (filters.include.is_empty() || matches_any(&filters.include, &link))
                && (!same_origin
                    || !filters.stay_below
                    || link.starts_with(&format!("{base_dir}/")));

            if remaining > 0 && may_explore && (same_origin || ext_left > 0) {
                // Same-site pages, and other sites while the "other sites" levels last, are crawled further
                queue.push_back((
                    link.clone(),
                    remaining - 1,
                    if same_origin { ext_left } else { ext_left - 1 },
                ));
            } else {
                // Cross-origin or leaf level — treat as a collectible link
                collected.push(link);
            }
        }
    }

    // Filter by extension if requested
    if !extensions.is_empty() {
        collected.retain(|link| {
            let path = link.split('?').next().unwrap_or("");
            let ext = path
                .split('.')
                .next_back()
                .unwrap_or("")
                .to_ascii_lowercase();
            extensions
                .iter()
                .any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext))
        });
    }

    Ok(collected)
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    #[test]
    fn patterns_match_without_scheme_and_case() {
        let p = vec![
            "*/logout*".to_string(),
            "files.example.com/private/*".to_string(),
        ];
        assert!(matches_any(&p, "https://x.test/Logout?next=1"));
        assert!(matches_any(&p, "http://files.example.com/private/a.zip"));
        assert!(!matches_any(&p, "http://files.example.com/public/a.zip"));
        assert!(!matches_any(&[String::new()], "http://x.test/"));
    }
}

/// Split a pattern field ("a, b c") into patterns.
pub fn split_patterns(text: &str) -> Vec<String> {
    text.split([',', '\n', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// What a re-crawl changed compared with the files a project already knew.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct ProjectDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

pub fn diff_results(old: &[String], new: &[String]) -> ProjectDiff {
    let old_set: HashSet<&String> = old.iter().collect();
    let new_set: HashSet<&String> = new.iter().collect();
    ProjectDiff {
        added: new
            .iter()
            .filter(|u| !old_set.contains(u))
            .cloned()
            .collect(),
        removed: old
            .iter()
            .filter(|u| !new_set.contains(u))
            .cloned()
            .collect(),
    }
}

/// Run a saved project's settings again (the window stores them as one JSON object).
pub async fn crawl_from_config(config: &serde_json::Value) -> Result<Vec<String>> {
    let text = |key: &str| {
        config
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let url = text("url");
    anyhow::ensure!(!url.trim().is_empty(), "the project has no start address");
    let depth = config
        .get("depth")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
        .min(5) as u8;
    let external = config
        .get("external_levels")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
        .min(5) as u8;
    let filters = GrabFilters {
        exclude: split_patterns(&text("exclude")),
        include: split_patterns(&text("include")),
        stay_below: config
            .get("stay_below")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        external_levels: external,
    };
    grab_links(
        url.trim(),
        &split_patterns(&text("extensions")),
        depth,
        &filters,
    )
    .await
}

#[cfg(test)]
mod project_tests {
    use super::*;

    #[test]
    fn diff_lists_new_and_vanished_files() {
        let old = vec!["a".to_string(), "b".to_string()];
        let new = vec!["b".to_string(), "c".to_string()];
        assert_eq!(
            diff_results(&old, &new),
            ProjectDiff {
                added: vec!["c".into()],
                removed: vec!["a".into()]
            }
        );
        assert_eq!(
            diff_results(&old, &old),
            ProjectDiff {
                added: vec![],
                removed: vec![]
            }
        );
    }

    #[test]
    fn pattern_fields_split_on_commas_spaces_and_lines() {
        assert_eq!(
            split_patterns("*/a*, */b* \n*/c*"),
            vec!["*/a*", "*/b*", "*/c*"]
        );
        assert!(split_patterns("  ").is_empty());
    }
}

fn extract_links(html: &str, base: &reqwest::Url) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let lower_bytes = lower.as_bytes();
    let mut results = Vec::new();
    let mut seen = HashSet::new();
    let mut i = 0;

    while i < lower_bytes.len() {
        let skip = if lower_bytes[i..].starts_with(b"href=") {
            Some(5)
        } else if lower_bytes[i..].starts_with(b"src=") {
            Some(4)
        } else {
            None
        };
        let Some(skip) = skip else {
            i += 1;
            continue;
        };
        i += skip;
        if i >= html.len() {
            break;
        }
        let (href, consumed) = parse_attr_value(&html[i..]);
        if let Some(href) = href {
            if let Ok(resolved) = base.join(&href) {
                let scheme = resolved.scheme();
                if (scheme == "http" || scheme == "https") && seen.insert(resolved.to_string()) {
                    results.push(resolved.to_string());
                }
            }
        }
        i += consumed.max(1);
    }

    results
}

fn parse_attr_value(s: &str) -> (Option<String>, usize) {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return (None, 0);
    }
    let quote = bytes[0];
    if quote == b'"' || quote == b'\'' {
        let end = bytes[1..].iter().position(|&b| b == quote).unwrap_or(0);
        let value = s[1..=end].to_string();
        return (if value.is_empty() { None } else { Some(value) }, end + 2);
    }
    let end = bytes
        .iter()
        .position(|&b| b == b' ' || b == b'>' || b == b'\n' || b == b'\t' || b == b'\r')
        .unwrap_or(bytes.len());
    if end > 0 {
        (Some(s[..end].to_string()), end)
    } else {
        (None, 0)
    }
}
