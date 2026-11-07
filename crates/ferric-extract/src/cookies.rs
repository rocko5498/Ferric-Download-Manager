//! Cookie helpers for extractors.
//!
//! Hard constraint: never read Chrome/Firefox cookie databases from disk.
//! Callers must pass cookies supplied by the browser extension (or none).

/// Single cookie name/value (optional domain for filtering helpers).
#[derive(Debug, Clone)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
}

/// Format cookies as a single `Cookie` header value.
pub fn to_header(cookies: &[Cookie]) -> String {
    cookies
        .iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Parse a `Cookie` header into name/value pairs (no disk I/O).
pub fn from_header(header: &str, domain: &str) -> Vec<Cookie> {
    header
        .split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let (name, value) = part.split_once('=')?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some(Cookie {
                name: name.to_string(),
                value: value.trim().to_string(),
                domain: domain.to_string(),
            })
        })
        .collect()
}

/// Intentionally empty: disk cookie DB access is forbidden.
/// Kept so call sites compile; always returns no cookies.
pub async fn read_cookies(_domain: &str) -> Vec<Cookie> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip_shape() {
        let cookies = from_header("a=1; b=two", "example.com");
        assert_eq!(cookies.len(), 2);
        assert_eq!(to_header(&cookies), "a=1; b=two");
    }

    #[tokio::test]
    async fn never_reads_disk() {
        assert!(read_cookies("example.com").await.is_empty());
    }
}
