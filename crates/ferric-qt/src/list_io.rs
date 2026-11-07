//! Export / import of the download list in two plain-text formats:
//! a text file with one URL per line, and an export file (.ef2) where each
//! entry is `<`, the URL, optional `referer:` and `User-Agent:` lines, then `>`; all CRLF.

pub struct Entry {
    pub url: String,
    pub referrer: Option<String>,
}

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)";

pub fn to_text(entries: &[Entry]) -> String {
    entries.iter().map(|e| format!("{}\r\n", e.url)).collect()
}

pub fn to_ef2(entries: &[Entry]) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str("<\r\n");
        out.push_str(&e.url);
        out.push_str("\r\n");
        if let Some(r) = e.referrer.as_deref().filter(|r| !r.is_empty()) {
            out.push_str(&format!("referer: {r}\r\n"));
        }
        out.push_str(&format!("User-Agent: {USER_AGENT}\r\n>\r\n"));
    }
    out
}

fn is_url(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://") || l.starts_with("ftp://")
}

/// Reads either file kind: an IDM export file (starts with `<`) or plain URL lines.
pub fn parse(text: &str) -> Vec<Entry> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    if lines.iter().find(|l| !l.is_empty()) != Some(&"<") {
        return lines
            .into_iter()
            .filter(|l| is_url(l))
            .map(|l| Entry {
                url: l.to_string(),
                referrer: None,
            })
            .collect();
    }
    let mut out = Vec::new();
    let mut cur: Option<Entry> = None;
    for line in lines {
        match line {
            "<" => {
                cur = Some(Entry {
                    url: String::new(),
                    referrer: None,
                })
            }
            ">" => {
                if let Some(e) = cur.take().filter(|e| is_url(&e.url)) {
                    out.push(e);
                }
            }
            _ => {
                if let Some(e) = cur.as_mut() {
                    if e.url.is_empty() {
                        e.url = line.to_string();
                    } else if let Some(r) = line
                        .strip_prefix("referer:")
                        .or_else(|| line.strip_prefix("Referer:"))
                    {
                        e.referrer = Some(r.trim().to_string());
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Entry> {
        vec![
            Entry {
                url: "http://a.test/x.zip".into(),
                referrer: Some("http://a.test/page".into()),
            },
            Entry {
                url: "ftp://b.test/y.bin".into(),
                referrer: None,
            },
        ]
    }

    #[test]
    fn text_export_is_one_crlf_url_per_line() {
        assert_eq!(
            to_text(&sample()),
            "http://a.test/x.zip\r\nftp://b.test/y.bin\r\n"
        );
    }

    #[test]
    fn export_file_round_trips_the_ef2_layout() {
        let ef2 = to_ef2(&sample());
        assert!(ef2.starts_with(
            "<\r\nhttp://a.test/x.zip\r\nreferer: http://a.test/page\r\nUser-Agent: "
        ));
        assert!(ef2.ends_with(">\r\n"));
        assert_eq!(ef2.matches("<\r\n").count(), 2);
    }

    #[test]
    fn both_file_kinds_read_back() {
        for text in [to_text(&sample()), to_ef2(&sample())] {
            let back = parse(&text);
            assert_eq!(back.len(), 2);
            assert_eq!(back[0].url, "http://a.test/x.zip");
            assert_eq!(back[1].url, "ftp://b.test/y.bin");
        }
        assert_eq!(
            parse(&to_ef2(&sample()))[0].referrer.as_deref(),
            Some("http://a.test/page")
        );
    }

    #[test]
    fn junk_lines_and_non_urls_are_skipped() {
        let back =
            parse("hello\r\n\r\nhttps://ok.test/f\r\njavascript:alert(1)\r\nfile:///c:/x\r\n");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].url, "https://ok.test/f");
    }
}
