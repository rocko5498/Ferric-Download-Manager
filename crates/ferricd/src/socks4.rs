//! SOCKS4 / `SOCKS4a` support for the HTTP clients.
//!
//! `reqwest` only speaks SOCKS5. IDM also supports SOCKS4(a), so a proxy URL
//! with scheme `socks4` / `socks4a` is served through a tiny loopback bridge:
//! reqwest talks SOCKS5 (`socks5h`, i.e. host names are sent, not resolved) to
//! the bridge, and the bridge repeats each CONNECT upstream as `SOCKS4a`.

use anyhow::{bail, Context, Result};
use std::net::Ipv4Addr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// `Some((host, port, userid))` when `proxy_url` is a SOCKS4/4a URL.
pub fn parse(proxy_url: &str) -> Option<(String, u16, String)> {
    let url = url::Url::parse(proxy_url).ok()?;
    if !matches!(url.scheme(), "socks4" | "socks4a") {
        return None;
    }
    Some((
        url.host_str()?.to_string(),
        url.port().unwrap_or(1080),
        url.username().to_string(),
    ))
}

/// Start the bridge for `proxy_url` (must be a SOCKS4/4a URL) and return the
/// `socks5h://127.0.0.1:<port>` URL reqwest should use instead. Needs a Tokio
/// runtime. The bridge lives as long as the runtime.
pub fn start_bridge(proxy_url: &str) -> Result<String> {
    let (host, port, user) = parse(proxy_url).context("not a socks4/socks4a proxy URL")?;
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    std_listener.set_nonblocking(true)?;
    let local = std_listener.local_addr()?;
    let handle = tokio::runtime::Handle::try_current()
        .context("SOCKS4 proxy needs a running Tokio runtime")?;
    let upstream = format!("{host}:{port}");
    handle.spawn(async move {
        let Ok(listener) = TcpListener::from_std(std_listener) else {
            return;
        };
        loop {
            let Ok((client, _)) = listener.accept().await else {
                break;
            };
            let upstream = upstream.clone();
            let user = user.clone();
            tokio::spawn(async move {
                let _ = bridge_one(client, &upstream, &user).await;
            });
        }
    });
    Ok(format!("socks5h://127.0.0.1:{}", local.port()))
}

async fn bridge_one(mut client: TcpStream, upstream: &str, user: &str) -> Result<()> {
    // --- SOCKS5 server side (no authentication) ---
    let mut hello = [0_u8; 2];
    client.read_exact(&mut hello).await?;
    let mut methods = vec![0_u8; hello[1] as usize];
    client.read_exact(&mut methods).await?;
    client.write_all(&[5, 0]).await?;

    let mut req = [0_u8; 4];
    client.read_exact(&mut req).await?;
    if req[0] != 5 || req[1] != 1 {
        client.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        bail!("only CONNECT is supported");
    }
    let (ip, domain) = match req[3] {
        1 => {
            let mut raw = [0_u8; 4];
            client.read_exact(&mut raw).await?;
            (Ipv4Addr::from(raw), None)
        }
        3 => {
            let mut len = [0_u8; 1];
            client.read_exact(&mut len).await?;
            let mut name = vec![0_u8; len[0] as usize];
            client.read_exact(&mut name).await?;
            // SOCKS4a: 0.0.0.x tells the proxy to resolve the trailing host name.
            (Ipv4Addr::new(0, 0, 0, 1), Some(name))
        }
        _ => {
            client.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            bail!("IPv6 targets are not supported by SOCKS4");
        }
    };
    let mut port = [0_u8; 2];
    client.read_exact(&mut port).await?;

    // --- SOCKS4(a) client side ---
    let mut up = TcpStream::connect(upstream).await?;
    let mut msg = vec![4_u8, 1];
    msg.extend_from_slice(&port);
    msg.extend_from_slice(&ip.octets());
    msg.extend_from_slice(user.as_bytes());
    msg.push(0);
    if let Some(name) = domain {
        msg.extend_from_slice(&name);
        msg.push(0);
    }
    up.write_all(&msg).await?;
    let mut reply = [0_u8; 8];
    up.read_exact(&mut reply).await?;
    if reply[1] != 0x5a {
        client.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        bail!("SOCKS4 proxy refused the connection (code {:#x})", reply[1]);
    }
    client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    let _ = tokio::io::copy_bidirectional(&mut client, &mut up).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_socks4_urls_only() {
        assert_eq!(
            parse("socks4a://bob@10.0.0.1:1081"),
            Some(("10.0.0.1".into(), 1081, "bob".into()))
        );
        assert_eq!(parse("socks4://h"), Some(("h".into(), 1080, String::new())));
        assert!(parse("socks5://h:1").is_none());
        assert!(parse("http://h:1").is_none());
    }
}
