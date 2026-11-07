//! Pure-Rust extraction engine (yt-dlp *personality*, not a line-by-line port).
//!
//! Design goals:
//! - Every site is an `InfoExtractor` (IE) with suitable URL matchers
//! - HTTP only through a shared `reqwest::Client` (caller applies security::validate_url)
//! - No shell, no embedded Python, no vendored yt-dlp binary
//! - Optional subprocess fallback is a separate module (`ytdlp`), off unless enabled

use anyhow::Result;
use ferric_common::ExtractResult;
use std::future::Future;
use std::pin::Pin;

/// Shared context for all IEs. Cookies are header-ready (`name=value; ...`), never logged here.
#[derive(Clone)]
pub struct ExtractContext {
    pub client: reqwest::Client,
    /// Optional Cookie header for login-walled sites.
    pub cookie_header: Option<String>,
    /// Prefer failing natively over shelling out (default true for pure-Rust mode).
    pub pure_rust_only: bool,
}

impl ExtractContext {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            cookie_header: None,
            pure_rust_only: true,
        }
    }
}

/// Boxed async result type for object-safe dispatch.
pub type ExtractFut<'a> = Pin<Box<dyn Future<Output = Result<ExtractResult>> + Send + 'a>>;

/// yt-dlp-style InfoExtractor.
pub trait InfoExtractor: Send + Sync {
    /// Stable IE key (e.g. `"Youtube"`, `"TedTalks"`) — mirrors yt-dlp extractor key style.
    fn key(&self) -> &'static str;

    /// True if this IE should handle `url`.
    fn suitable(&self, url: &str) -> bool;

    /// Extract metadata + formats. Must not panic on bad HTML.
    fn extract<'a>(&'a self, url: &'a str, ctx: &'a ExtractContext) -> ExtractFut<'a>;
}

/// Registry entry for static dispatch table.
pub struct IeEntry {
    pub key: &'static str,
    pub suitable: fn(&str) -> bool,
    pub extract: for<'a> fn(&'a str, &'a ExtractContext) -> ExtractFut<'a>,
}
