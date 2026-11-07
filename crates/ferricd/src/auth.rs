//! Per-job HTTP authentication shared by every connection of a download.
//!
//! IDM sends `Authorization: Basic` on the very first request and every later
//! connection when the URL carries credentials, and answers a Digest challenge
//! once, then reuses the nonce with an increasing `nc` on every request. This
//! type reproduces that so ranged connections do not each hit a 401 (or, worse,
//! lose the credentials and drop the download to a single stream).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub struct AuthState {
    user: String,
    pass: String,
    /// Last `WWW-Authenticate` challenge we answered (`None` = send Basic).
    challenge: Mutex<Option<String>>,
    /// Digest request counter, unique across all connections of the job.
    nc: AtomicU32,
    cnonce: String,
}

impl AuthState {
    /// Credentials from `http://user:pass@host/...`, if any.
    pub fn from_url(url: &str) -> Option<Arc<Self>> {
        let (user, pass) = crate::chunk::credentials_from_url(url)?;
        Some(Arc::new(Self {
            user,
            pass,
            challenge: Mutex::new(None),
            nc: AtomicU32::new(0),
            cnonce: format!("{:08x}", rand::random::<u32>()),
        }))
    }

    /// Remember a server challenge. Returns `true` when it differs from the one
    /// already in use, i.e. repeating the request with a new header can succeed.
    pub fn learn_challenge(&self, www_authenticate: &str) -> bool {
        let mut slot = self
            .challenge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.as_deref() == Some(www_authenticate) {
            return false;
        }
        *slot = Some(www_authenticate.to_string());
        true
    }

    /// Adopt the challenge the planner already answered with `nc=1`, so the
    /// first ranged request continues with `nc=2` (a nonce/nc pair is never reused).
    pub fn adopt_plan_challenge(&self, www_authenticate: &str) {
        self.learn_challenge(www_authenticate);
        self.nc.store(1, Ordering::SeqCst);
    }

    /// `Authorization` header value for one request.
    pub fn header(&self, method: &str, url: &str) -> String {
        let challenge = self
            .challenge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        match challenge {
            Some(c) if is_digest(&c) => {
                let nc = self.nc.fetch_add(1, Ordering::SeqCst) + 1;
                crate::chunk::digest_header(
                    &c,
                    method,
                    url,
                    &self.user,
                    &self.pass,
                    nc,
                    &self.cnonce,
                )
            }
            _ => crate::chunk::basic_auth_value(&self.user, &self.pass),
        }
    }
}

fn is_digest(www_authenticate: &str) -> bool {
    www_authenticate
        .split_whitespace()
        .next()
        .is_some_and(|s| s.eq_ignore_ascii_case("digest"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_credentials_in_url_means_no_state() {
        assert!(AuthState::from_url("http://example.com/file").is_none());
    }

    #[test]
    fn basic_is_sent_without_a_challenge() {
        let a = AuthState::from_url("http://user:pass@example.com/f").unwrap();
        assert_eq!(
            a.header("GET", "http://example.com/f"),
            "Basic dXNlcjpwYXNz"
        );
    }

    #[test]
    fn digest_nc_increases_per_request() {
        let a = AuthState::from_url("http://user:pass@example.com/f?x=1").unwrap();
        assert!(a.learn_challenge(r#"Digest realm="r", nonce="n", qop="auth""#));
        assert!(!a.learn_challenge(r#"Digest realm="r", nonce="n", qop="auth""#));
        let h1 = a.header("GET", "http://example.com/f?x=1");
        let h2 = a.header("GET", "http://example.com/f?x=1");
        assert!(
            h1.contains("nc=00000001") && h1.contains(r#"uri="/f?x=1""#),
            "{h1}"
        );
        assert!(h2.contains("nc=00000002"), "{h2}");
        assert!(h1.contains("qop=auth") && h1.contains("cnonce="));
    }
}
