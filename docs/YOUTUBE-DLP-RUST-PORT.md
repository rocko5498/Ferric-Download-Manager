# Pure-Rust media extraction (yt-dlp *ideas*, not Python)

**Authoritative product plan:** Ferric extraction is **Rust**.  
Optional system yt-dlp binary is documented in `YTDLP-INTEGRATION.md` as opt-in only.

This file tracks the **ongoing pure-Rust IE port** — the work we continue.

## Architecture

```
URL
 → extract_with_config()          # pure Rust first
 → REGISTRY / playlist / generic  # Rust IEs
 → ExtractResult
 → selector::select()             # yt-dlp-style format grammar (Rust)
 → info_json::to_ytdlp_info()     # interoperable export (Rust)
 → ferricd download engines         # Rust
```

Optional (settings + feature): `ytdlp` module runs **standalone yt-dlp binary** — never embeds Python.

## IE inventory (pure Rust)

| Key | Module | Notes |
|-----|--------|--------|
| youtube | youtube.rs | rusty_ytdl (Rust crate) + timedtext/chapters |
| vimeo, reddit, twitter, tiktok, soundcloud, twitch | * | HTTP/JSON |
| dailymotion, bandcamp, bilibili | * | |
| instagram, facebook | * | cookies/CDP-oriented |
| pinterest, imgur, vk, rumble, odysee | * | |
| kick, espn, niconico, crunchyroll, hotstar | * | best-effort |
| bbc_iplayer, arte, zdf, nhk, ted | * | |
| archive, streamable, bitchute, peertube, okru | * | |
| loom, mixcloud, newgrounds | * | continued port |
| generic | generic.rs | HTML + embed delegation to REGISTRY |

Count: `ferric-extract::REGISTRY.len()` / `GET /media/ytdlp`.

## Safety

1. Daemon: `security::validate_url()` on user URLs  
2. Pure-Rust path: no shell, no Python  
3. Optional binary: argv only, allowlist, caps  
4. Cookies never logged  
5. No `unwrap`/`expect` on production paths  

## Roadmap (continue)

| Area | Status |
|------|--------|
| Format selector grammar | Done |
| info.json export | Done |
| Chapters / subs (YT) | Done |
| ~35 site IEs | In progress — **keep growing** |
| Full 1800-site 1:1 | Not a deadline; port by demand |
| Replace rusty_ytdl with first-party YT | Future |
| DRM | Out of scope |

## Build

```bash
# Product pure-Rust extract (default path)
cargo build -p ferric-extract
cargo build -p ferricd

# Optional binary bridge still available as feature
cargo build -p ferricd --features ytdlp-subprocess
```
