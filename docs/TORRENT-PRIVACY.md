# Ferric — Torrent Privacy Threat Model & Hardening Specification

## Overview

Unlike HTTP downloads, where data is fetched directly from centralized content delivery networks (CDNs) or web hosts, the BitTorrent protocol is fundamentally collaborative. Peers must discover each other, request chunks of files from one another, and announce their presence to coordinate swarms.

This collaborative model introduces major vector threats to user privacy. Under default configurations, a standard BitTorrent client exposes the user to immediate network surveillance, location tracking, local network exposure, and passive IP logging.

This specification outlines the **10 Threat Axes** targeting Torrent downloads within Ferric, defines the attack mechanics, and establishes the mitigation policies enforced by the Ferric download daemon (`ferricd`).

---

## 10 Threat Axes & Mitigation Matrix

| ID | Threat Vector | Attack Mechanics | Ferric Mitigation | Anonymity Weight |
|:---|:---|:---|:---|:---:|
| **T1** | **Tracker Peer-List Scraping** | Surveillance firms crawl public tracker `/announce` endpoints, registering as dummy peers to scrape the complete list of active peer IPs for a given `info_hash`. | **Proxy Gate Enforcement:** Block public trackers unless a verified SOCKS5/VPN proxy is active. | +40 points |
| **T2** | **DHT IP Broadcast** | Mainline DHT queries broadcast the user's IP to lookup nodes. Malicious crawlers crawl DHT get_peers transactions to log IPs downloading specific hashes. | **DHT Disabled by Default:** `disable_dht: true` and `disable_dht_persistence: true` for all non-private swarms. | +15 points |
| **T3** | **PEX Swarm Redistribution** | Peer Exchange (PEX) allows connected peers to exchange addresses. A single honeypot node in a swarm queries all peers for their neighbors, mapping the swarm. | **PEX Auto-Block:** Automatic blocklist filtering + upstream PEX disable option for public torrents. | +10 points |
| **T4** | **LSD Local Broadcast** | Local Service Discovery (LSD) broadcasts SSDP/multicast UDP packets across the LAN to announce active downloads, exposing files to cafe/office networks. | **LSD Disabled by Default:** Disable local discovery; block local network announcements. | +5 points |
| **T5** | **Swarm Honeypot Surveillance** | Copyright enforcement nodes join public swarms, handshake with active peers, and record connection logs as proof of sharing. | **IP Blocklist Integration:** Auto-ingestion of known tracking blocklists via `blocklist_url`. | +10 points |
| **T6** | **Magnet Metadata Pre-Fetch** | Resolving magnet links requires fetching the torrent file from peers via DHT metadata extensions, exposing user IP before the user presses "Start". | **Proxy-First Resolution:** Enforce proxy routing on metadata resolution; alert user before fetching. | +5 points |
| **T7** | **DNS Leaks** | Resolving tracker hostnames through default system DNS leaks target info_hashes to local ISPs or routers, even if peer traffic is proxied. | **Remote DNS Verification:** Verify SOCKS5 hostname routing to enforce remote resolution. | +10 points |
| **T8** | **Seeding Tail Exposure** | Leaving a client seeding after download completion leaves the user IP in the swarm indefinitely, maximizing exposure time. | **Default Seeding Off:** Seeding disabled by default; auto-stop rules based on ratio/time. | +10 points |
| **T9** | **DPI Handshake Fingerprinting** | ISPs monitor traffic patterns and TCP headers to detect the BitTorrent protocol handshake (`\x13BitTorrent protocol`), throttling or logging the connection. | **Handshake Obfuscation:** Protocol encryption (MSE/PE) + handshake jitter to mask traffic. | +10 points (Future) |
| **T10** | **Persistent peer_id Tracking** | Clients generate a unique 20-byte `peer_id`. If persistent across sessions, trackers and peers can correlate different downloads to the same client. | **Rotational Peer ID:** Enforce `peer_id: None` (librqbit randomizes the ID on every session). | +5 points |

---

## Technical Breakdown of Threats & Attack Mechanics

### T1: Tracker Peer-List Scraping
Trackers act as matchmakers for swarms. When a client announces to a tracker via HTTP or UDP, it receives a list of other peers in the swarm. Attackers do not need to download the torrent content; they simply send periodic `/announce` requests to the tracker for popular `info_hash` values. The tracker responds with lists of dozens of active IP addresses. This allows passive collection of peer databases without joining the swarm directly.
*   **Mitigation:** Ferric routes all tracker communication through SOCKS5 proxies or VPN interfaces.

### T2: DHT IP Broadcast & Sybil Attacks
The Mainline DHT (Distributed Hash Table) is a decentralized network using Kademlia. Nodes route queries to find peers downloading a specific torrent (`info_hash`).
*   **Academic Proof:** In *"Real-world Sybil Attacks in BitTorrent Mainline DHT"* (IEEE 2013), researchers documented how attackers deploy hundreds of thousands of "Sybil" nodes (fake nodes controlled by a single entity). These Sybils position themselves strategically in the Kademlia keyspace to intercept search queries for specific target info_hashes. When a legitimate client queries the DHT for a torrent, it hits one of these Sybil nodes, which immediately logs the client's IP address.
*   **Mitigation:** Ferric sets `disable_dht: true` for public torrents unless explicitly enabled, and clears DHT state on client exit via `disable_dht_persistence: true`.

### T3: Peer Exchange (PEX) Swarm Mapping
PEX allows peers to directly ask other peers: *"Who else are you connected to?"* This accelerates swarm discovery without hitches, but creates a surveillance vector. A single monitoring peer can establish a TCP connection with a user's client, request their PEX updates, and harvest the IP addresses of all other peers the user is connected to.
*   **Mitigation:** Ferric respects the `.torrent` `private` flag, which auto-disables PEX. For public torrents, Ferric supports a flag to disable PEX redistribution entirely.

### T4: Local Service Discovery (LSD) Leaks
LSD broadcasts multicast packets (typically UDP port 6771) on the local subnet. It sends XML or plain text headers announcing: *"I am IP X.X.X.X, and I am downloading info_hash Y."* Any device on the same Wi-Fi or LAN segment (e.g., in a public café or corporate network) can capture these multicast packets and identify exactly what the user is downloading.
*   **Mitigation:** Local discovery is explicitly disabled in the engine configuration.

### T5: Swarm Honeypots
Surveillance agents run customized BitTorrent clients that join swarms and actively handshake with other peers. Once a handshake completes, they log the timestamp, peer IP, port, and client version as forensic evidence of copy sharing.
*   **Mitigation:** Ferric ingests blocklist formats (e.g., IP range databases) via the `blocklist_url` session option in the daemon, refusing connections from listed IP blocks.

### T6: Magnet Link Metadata Exposure
Magnet links do not contain file metadata; they only contain the `info_hash`. To show the user the file tree, the client must announce the hash to trackers and the DHT to find peers holding the `.torrent` metadata. Doing this leaks the user's interest in the hash before they ever confirm the download.
*   **Mitigation:** Magnet links are resolved exclusively through the proxy interface. If no proxy is set, Ferric warns the user and requests confirmation before performing the DHT lookup.

### T7: DNS Leaks
Even if peer-to-peer data traffic is sent through a SOCKS5 proxy, standard DNS resolution of tracker hostnames (e.g., `tracker.example.com`) can default to the system's local DNS server. This reveals to the local ISP that the user's computer queried a tracker domain name.
*   **Mitigation:** Ferric verifies that the SOCKS5 proxy configuration delegates hostname resolution to the proxy endpoint (remote DNS resolution), preventing local queries.

### T8: Seeding tail Exposure
Seeding keeps the user active in the swarm. The longer the client seeds, the higher the probability that a surveillance scanner or crawler logs their IP.
*   **Mitigation:** Seeding is disabled by default. The user can configure auto-stop conditions:
    1.  **Ratio Limit:** Stop seeding when upload/download ratio hits 1.0 (or a custom limit).
    2.  **Time Limit:** Stop seeding N minutes after download completion.
    3.  **Seeders Count:** Stop seeding if the swarm already has >50 healthy seeders.

### T9: Deep Packet Inspection (DPI)
Network operators inspect traffic for the standard BitTorrent protocol handshake. If identified, they shape or terminate TCP/UDP traffic.
*   **Mitigation:** Support Message Stream Encryption (MSE/PE) and insert packet size jitter to obscure protocol patterns.

### T10: Persistent peer_id
The `peer_id` is a 20-byte identifier sent to trackers and peers. If static, it behaves like a cookie, identifying the client across different networks.
*   **Mitigation:** Ferric enforces rotational peer IDs, generating a new, cryptographically random `peer_id` every time the daemon session starts.

---

## The Ferric Anonymity Score

To make these protections visible, Ferric implements a real-time **Anonymity Score** (0–100) calculated on the settings page:

$$\text{Anonymity Score} = S_{\text{Proxy}} + S_{\text{DNS}} + S_{\text{DHT}} + S_{\text{PEX}} + S_{\text{LSD}} + S_{\text{Seed}} + S_{\text{Private}} + S_{\text{ID}}$$

Where:
*   $S_{\text{Proxy}} = 40$: Verified active SOCKS5/VPN proxy.
*   $S_{\text{DNS}} = 10$: DNS resolved remotely through proxy.
*   $S_{\text{DHT}} = 15$: DHT disabled in settings.
*   $S_{\text{PEX}} = 10$: PEX disabled or private-only.
*   $S_{\text{LSD}} = 5$: LSD local network discovery disabled.
*   $S_{\text{Seed}} = 10$: Default seeding off / auto-stop enabled.
*   $S_{\text{Private}} = 5$: Using private trackers or tracker-less configs.
*   $S_{\text{ID}} = 5$: `peer_id` rotation enabled.

### Grading Scale:
*   **A (80–100):** Comprehensive privacy protection. Enforced SOCKS5, disabled DHT/LSD, rotational IDs.
*   **B (60–79):** Moderate protection. Proxied, but some swarm protocols (DHT/PEX) enabled.
*   **C (40–59):** Weak protection. Proxy active, but local leaks or seeding tail left unmitigated.
*   **D (20–39):** Dangerous. No proxy, relying purely on protocol-level disabling.
*   **F (0–19):** High Risk. Default client configuration with public trackers, DHT, and LSD active. Displays a pulsing red warning.
