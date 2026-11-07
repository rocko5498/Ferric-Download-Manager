//! Netscape cookies.txt import/export (yt-dlp compatible) — Task 111.

use anyhow::{bail, Result};
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetscapeCookie {
    pub domain: String,
    pub include_subdomains: bool,
    pub path: String,
    pub secure: bool,
    pub expires: i64,
    pub name: String,
    pub value: String,
}

/// Parse Netscape cookie file text into structured cookies.
pub fn parse(text: &str) -> Result<Vec<NetscapeCookie>> {
    let mut out = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            // HTTP-only prefix: #HttpOnly_.example.com ...
            if let Some(rest) = line.strip_prefix("#HttpOnly_") {
                if let Some(c) = parse_line(rest, true) {
                    out.push(c);
                }
            }
            continue;
        }
        match parse_line(line, false) {
            Some(c) => out.push(c),
            None => bail!("invalid cookie line {}: {line}", lineno + 1),
        }
    }
    Ok(out)
}

fn parse_line(line: &str, _http_only: bool) -> Option<NetscapeCookie> {
    let parts: Vec<&str> = line.split('\t').collect();
    if parts.len() < 7 {
        return None;
    }
    Some(NetscapeCookie {
        domain: parts[0].to_string(),
        include_subdomains: parts[1].eq_ignore_ascii_case("TRUE"),
        path: parts[2].to_string(),
        secure: parts[3].eq_ignore_ascii_case("TRUE"),
        expires: parts[4].parse().ok()?,
        name: parts[5].to_string(),
        value: parts[6..].join("\t"),
    })
}

/// Serialize cookies to Netscape cookies.txt format.
pub fn export(cookies: &[NetscapeCookie]) -> String {
    let mut s =
        String::from("# Netscape HTTP Cookie File\n# https://curl.se/docs/http-cookies.html\n\n");
    for c in cookies {
        let _ = writeln!(
            s,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            c.domain,
            if c.include_subdomains {
                "TRUE"
            } else {
                "FALSE"
            },
            c.path,
            if c.secure { "TRUE" } else { "FALSE" },
            c.expires,
            c.name,
            c.value
        );
    }
    s
}

/// Build a Cookie request header for `domain` from a cookie jar.
pub fn header_for_domain(cookies: &[NetscapeCookie], host: &str) -> Option<String> {
    let host = host.to_ascii_lowercase();
    let mut pairs = Vec::new();
    for c in cookies {
        let dom = c.domain.trim_start_matches('.').to_ascii_lowercase();
        let ok = if c.include_subdomains || c.domain.starts_with('.') {
            host == dom || host.ends_with(&format!(".{dom}"))
        } else {
            host == dom
        };
        if ok {
            pairs.push(format!("{}={}", c.name, c.value));
        }
    }
    if pairs.is_empty() {
        None
    } else {
        Some(pairs.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let raw = "# Netscape HTTP Cookie File\n\
.example.com\tTRUE\t/\tFALSE\t0\tsession\tabc123\n";
        let cookies = parse(raw).unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "session");
        let out = export(&cookies);
        assert!(out.contains("session\tabc123"));
        assert_eq!(
            header_for_domain(&cookies, "www.example.com").as_deref(),
            Some("session=abc123")
        );
    }
}
