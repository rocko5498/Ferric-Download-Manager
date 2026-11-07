//! Proxy auto-config (PAC): the `FindProxyForURL(url, host)` script IDM picks up from Internet Explorer's
//! settings. The script runs in a small JavaScript engine (boa) with the standard PAC helper functions.
//!
//! Limits: `dnsResolve`/`isResolvable`/`isInNet` only know literal IP addresses (a script run must not block
//! on DNS lookups), `myIpAddress` is `127.0.0.1`, and `dateRange` always answers true.
use anyhow::{anyhow, Result};
use boa_engine::{Context, Source};

#[derive(Debug, PartialEq, Eq)]
pub enum PacChoice {
    Direct,
    /// A proxy URL reqwest understands (`http://host:port`, `https://...`, `socks5://...`).
    Proxy(String),
}

const PRELUDE: &str = r#"
function isPlainHostName(host) { return host.indexOf('.') < 0; }
function dnsDomainIs(host, domain) {
  return host.length >= domain.length && host.substring(host.length - domain.length) === domain;
}
function localHostOrDomainIs(host, hostdom) {
  return host === hostdom || hostdom.lastIndexOf(host + '.', 0) === 0;
}
function dnsDomainLevels(host) { return host.split('.').length - 1; }
function shExpMatch(str, shexp) {
  var re = shexp.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.');
  return new RegExp('^' + re + '$').test(str);
}
function dnsResolve(host) {
  return /^\d+\.\d+\.\d+\.\d+$/.test(host) ? host : null;
}
function isResolvable(host) { return dnsResolve(host) !== null; }
function myIpAddress() { return '127.0.0.1'; }
function isInNet(host, pattern, mask) {
  var ip = dnsResolve(host);
  if (ip === null) return false;
  function num(s) { var p = s.split('.'); return ((+p[0] << 24) >>> 0) + (+p[1] << 16) + (+p[2] << 8) + (+p[3]); }
  return ((num(ip) & num(mask)) >>> 0) === ((num(pattern) & num(mask)) >>> 0);
}
function weekdayRange(wd1, wd2, gmt) {
  var days = ['SUN','MON','TUE','WED','THU','FRI','SAT'];
  var now = new Date(); var d = (gmt === 'GMT') ? now.getUTCDay() : now.getDay();
  var a = days.indexOf(wd1); var b = (wd2 && wd2 !== 'GMT') ? days.indexOf(wd2) : a;
  return a <= b ? (d >= a && d <= b) : (d >= a || d <= b);
}
function timeRange() {
  var args = Array.prototype.slice.call(arguments); var gmt = false;
  if (args.length && args[args.length - 1] === 'GMT') { gmt = true; args.pop(); }
  var now = new Date();
  var h = gmt ? now.getUTCHours() : now.getHours();
  var m = gmt ? now.getUTCMinutes() : now.getMinutes();
  var s = gmt ? now.getUTCSeconds() : now.getSeconds();
  var t = h * 3600 + m * 60 + s;
  if (args.length === 1) return h === args[0];
  if (args.length === 2) return h >= args[0] && h < args[1];
  if (args.length === 4) return t >= args[0] * 3600 + args[1] * 60 && t <= args[2] * 3600 + args[3] * 60;
  if (args.length === 6) return t >= args[0] * 3600 + args[1] * 60 + args[2] && t <= args[3] * 3600 + args[4] * 60 + args[5];
  return true;
}
function dateRange() { return true; }
function alert() {}
"#;

/// The automatic configuration script address Windows (Internet Explorer / Edge settings) has for this user,
/// if any: `AutoConfigURL` in the Internet Settings key.
#[cfg(windows)]
pub fn windows_pac_url() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    let url: String = key.get_value("AutoConfigURL").ok()?;
    let url = url.trim().to_string();
    (url.starts_with("http://") || url.starts_with("https://")).then_some(url)
}

#[cfg(not(windows))]
pub fn windows_pac_url() -> Option<String> {
    None
}

/// Run `script` for `url` and return the proxies it names, in order.
pub fn find_proxy(script: &str, url: &str) -> Result<Vec<PacChoice>> {
    let parsed = url::Url::parse(url)?;
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    let mut ctx = Context::default();
    ctx.eval(Source::from_bytes(PRELUDE))
        .map_err(|e| anyhow!("PAC helpers: {e}"))?;
    ctx.eval(Source::from_bytes(script))
        .map_err(|e| anyhow!("PAC script: {e}"))?;
    let call = format!(
        "FindProxyForURL({}, {})",
        serde_json::to_string(url)?,
        serde_json::to_string(&host)?
    );
    let value = ctx
        .eval(Source::from_bytes(&call))
        .map_err(|e| anyhow!("FindProxyForURL: {e}"))?;
    let text = value
        .to_string(&mut ctx)
        .map_err(|e| anyhow!("FindProxyForURL result: {e}"))?
        .to_std_string_escaped();
    Ok(parse_result(&text))
}

/// `"PROXY a:80; SOCKS b:1080; DIRECT"` -> the choices (unknown entries are skipped).
pub fn parse_result(text: &str) -> Vec<PacChoice> {
    text.split(';')
        .filter_map(|entry| {
            let entry = entry.trim();
            let (kind, rest) = entry.split_once(char::is_whitespace).unwrap_or((entry, ""));
            let rest = rest.trim();
            match kind.to_ascii_uppercase().as_str() {
                "DIRECT" => Some(PacChoice::Direct),
                "PROXY" | "HTTP" if !rest.is_empty() => {
                    Some(PacChoice::Proxy(format!("http://{rest}")))
                }
                "HTTPS" if !rest.is_empty() => Some(PacChoice::Proxy(format!("https://{rest}"))),
                "SOCKS" | "SOCKS5" if !rest.is_empty() => {
                    Some(PacChoice::Proxy(format!("socks5://{rest}")))
                }
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r#"
function FindProxyForURL(url, host) {
  if (isPlainHostName(host) || host === "localhost") return "DIRECT";
  if (dnsDomainIs(host, ".corp.example")) return "PROXY corp-proxy:3128; DIRECT";
  if (shExpMatch(url, "*://cdn.*/*.zip")) return "SOCKS5 127.0.0.1:1080";
  if (isInNet(host, "10.0.0.0", "255.0.0.0")) return "DIRECT";
  return "PROXY fallback.example:8080";
}
"#;

    #[test]
    fn script_picks_the_proxy_for_each_address() -> Result<()> {
        assert_eq!(
            find_proxy(SCRIPT, "http://intranet/x")?,
            vec![PacChoice::Direct]
        );
        assert_eq!(
            find_proxy(SCRIPT, "https://files.corp.example/a.bin")?,
            vec![
                PacChoice::Proxy("http://corp-proxy:3128".into()),
                PacChoice::Direct
            ]
        );
        assert_eq!(
            find_proxy(SCRIPT, "https://cdn.site.test/a.zip")?,
            vec![PacChoice::Proxy("socks5://127.0.0.1:1080".into())]
        );
        assert_eq!(
            find_proxy(SCRIPT, "http://10.1.2.3/a")?,
            vec![PacChoice::Direct]
        );
        assert_eq!(
            find_proxy(SCRIPT, "http://example.org/a")?,
            vec![PacChoice::Proxy("http://fallback.example:8080".into())]
        );
        Ok(())
    }

    #[test]
    fn broken_scripts_are_errors_not_panics() {
        assert!(find_proxy("function FindProxyForURL(", "http://a.test/").is_err());
        assert!(find_proxy("var x = 1;", "http://a.test/").is_err());
    }

    #[test]
    fn windows_script_address_is_none_or_an_http_url() {
        // Reads this machine's own setting: whatever it is, it must be absent or a usable address.
        if let Some(url) = windows_pac_url() {
            assert!(
                url.starts_with("http://") || url.starts_with("https://"),
                "{url}"
            );
        }
    }

    #[test]
    fn result_text_is_parsed() {
        assert_eq!(
            parse_result("PROXY a:80 ; SOCKS b:1080;BOGUS x; DIRECT"),
            vec![
                PacChoice::Proxy("http://a:80".into()),
                PacChoice::Proxy("socks5://b:1080".into()),
                PacChoice::Direct
            ]
        );
        assert!(parse_result("").is_empty());
    }
}
