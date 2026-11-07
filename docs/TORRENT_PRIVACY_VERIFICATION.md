# Torrent Privacy Verification

## Current Decision

Ferric must not treat rqbit SOCKS proxy support as leak-safe for privacy-required torrents yet.
The current daemon behavior is intentionally conservative:

- A torrent with `policy.proxy_required = true` refuses to start without an assigned proxy.
- Any torrent with an assigned proxy refuses rqbit start until proxy routing, DNS-over-proxy, UDP tracker behavior, and IPv6 suppression are packet-verified.
- Proxy-assigned rqbit starts are allowed only when `FERRIC_RQBIT_PRIVACY_BOUNDARY=external-verified`, `FERRIC_RQBIT_ENDPOINT` points at an already constrained external rqbit API, and `FERRIC_RQBIT_PRIVACY_RESULT` points at a passing verification JSON artifact for the assigned proxy.
- Non-privacy-required torrents may use the rqbit sidecar path for local/non-sensitive QA.

This avoids the dangerous state where the UI shows "proxy configured" while the engine leaks DNS, UDP tracker traffic, IPv6, DHT, PEX, or peer connections outside the proxy/VPN path.

## Why rqbit Is Blocked For Privacy Mode

Upstream rqbit has reported SOCKS leak paths involving UDP trackers and DNS resolution. Its public README documents `--socks-url`, but that alone is not enough for Ferric's privacy mode because the leak model includes more than TCP peer sockets.

Libtorrent's settings model is the minimum bar Ferric should expect from any engine: hostname lookups, tracker connections, and peer connections are separate proxy controls, with forced proxy behavior needed to fail closed. If an engine cannot expose equivalent controls, Ferric needs an outer network boundary instead.

The safe product decision is not to enable proxy-assigned starts until one of these is true:

1. Ferric uses an engine mode that can enforce all torrent egress through a verified proxy path.
2. Ferric runs the sidecar inside a network namespace/container/firewall rule set where non-proxy egress is impossible.
3. Ferric binds the engine to a verified VPN interface and packet capture proves no traffic leaves the real interface.

## External Verified Boundary

The managed rqbit sidecar remains blocked for proxy-assigned torrents. It cannot prove DNS-over-proxy, UDP tracker behavior, DHT/PEX/LSD, or IPv6 suppression.

For lab work only, Ferric can talk to a loopback rqbit API that was started outside the daemon inside a verified boundary:

```powershell
$env:FERRIC_RQBIT_ENDPOINT = "http://127.0.0.1:3030"
$env:FERRIC_RQBIT_PRIVACY_BOUNDARY = "external-verified"
$env:FERRIC_RQBIT_PRIVACY_RESULT = "R:\Rust Project\Rust.net Download Manager\diagnostics\torrent-privacy-result.json"
```

Use this only after the external rqbit process is constrained by a firewall, container, network namespace, or VPN-interface binding that fails closed. Do not set this for the daemon-managed sidecar. `FERRIC_RQBIT_ENDPOINT` must stay `http://localhost/...`, `http://127.0.0.1/...`, or another loopback IP; remote/LAN rqbit APIs are refused.
The JSON result must report `schema_version: 1`, `passed: true`, `packet_verified: true`, no connection or packet leaks, at least 30 seconds of capture, a `checked_at` timestamp no older than 15 minutes, the same proxy host/port assigned to the torrent, and the same rqbit endpoint as `FERRIC_RQBIT_ENDPOINT`.

## Windows Verification

Run this only with legal/local test torrents or a controlled tracker.

```powershell
.\scripts\Verify-TorrentPrivacy.ps1 `
  -DurationSeconds 60 `
  -ProxyHost 127.0.0.1 `
  -ProxyPort 1080 `
  -CapturePath diagnostics\torrent-privacy.pcapng `
  -ResultPath diagnostics\torrent-privacy-result.json `
  -RqbitEndpoint http://127.0.0.1:3030
```

To run connection table verification without elevated privileges (if `tshark`/Npcap is missing or cannot run due to UAC policies):

```powershell
.\scripts\Verify-TorrentPrivacy.ps1 `
  -DurationSeconds 60 `
  -ProxyHost 127.0.0.1 `
  -ProxyPort 1080 `
  -ResultPath diagnostics\torrent-privacy-result.json `
  -RqbitEndpoint http://127.0.0.1:3030 `
  -BypassPacketCheck
```

The script samples Windows connection tables for `ferricd` and `rqbit` and fails if it sees:

- TCP connections to any non-loopback remote other than the configured proxy.
- UDP endpoints bound outside loopback.

With `tshark` and `-CapturePath`, the script also analyzes captured packets and fails unless every non-loopback packet is to/from the configured proxy port.
Without packet analysis, the JSON artifact does not pass unless `-BypassPacketCheck` is explicitly requested.
If `-ResultPath` is provided, it writes a JSON pass/fail artifact with sampled PIDs, proxy endpoint, connection leaks, packet leaks, and capture path.

## Linux Verification

Run this only with legal/local test torrents or a controlled tracker.

```bash
./scripts/verify-torrent-privacy.sh \
  --duration 60 \
  --proxy-host 127.0.0.1 \
  --proxy-port 1080 \
  --capture-path diagnostics/torrent-privacy.pcap \
  --result-path diagnostics/torrent-privacy-result.json \
  --rqbit-endpoint http://127.0.0.1:3030
```

To run connection table verification without elevated privileges (if `tcpdump`/`tshark` is missing or cannot run):

```bash
./scripts/verify-torrent-privacy.sh \
  --duration 60 \
  --proxy-host 127.0.0.1 \
  --proxy-port 1080 \
  --result-path diagnostics/torrent-privacy-result.json \
  --rqbit-endpoint http://127.0.0.1:3030 \
  --bypass-packet-check
```

The script samples `ss` for `ferricd`/`rqbit` sockets. With `tcpdump` and `tshark`, it records and analyzes packets the same way as the Windows script: every non-loopback packet must be to/from the configured proxy port. Without packet analysis, the JSON artifact does not pass unless `--bypass-packet-check` is explicitly requested.

## Acceptance Bar

Task 053 is not complete until:

- `proxy_required` torrents can start only in a verified network mode.
- DNS resolution for tracker hostnames is proven to happen through the proxy/VPN path or not at all.
- UDP trackers, DHT, PEX, and LSD are disabled or forced through a verified path.
- IPv6 cannot bypass the selected route.
- Packet capture shows zero peer/tracker packets on the real interface.

## Live QA Gate

Task 052 live rqbit QA may proceed only for non-privacy-required local/legal torrents until this document's acceptance bar is met.

## Research Sources

- https://github.com/ikatson/rqbit/issues/493
- https://github.com/ikatson/rqbit
- https://github.com/qbittorrent/qBittorrent/issues/1894
- https://www.libtorrent.org/reference-Settings.html
- https://github.com/qbittorrent/qBittorrent/wiki/How-to-bind-your-vpn-to-prevent-ip-leaks
