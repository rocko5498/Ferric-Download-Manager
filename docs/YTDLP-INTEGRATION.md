# Extraction strategy (pure Rust first)

## One-line pitch

**Ferric is a Rust product end-to-end.**  
Media extraction lives in **`ferric-extract` (pure Rust)**.  
There is **no Python** in the app, no pip, no embedded CPython.

Optional **yt-dlp standalone binary** (GitHub release `.exe`/binary) is only an **opt-in coverage bridge** — not the product, not a Python dependency, and **not** the long-term plan.

---

## Authoritative plan (user)

| Priority | What |
|----------|------|
| **1. Primary** | Pure-Rust InfoExtractors in `ferric-extract` — **continue this work** |
| **2. Product** | Ferric download engines (chunked HTTP/2, multi-source, HLS/DASH, torrents, UI) |
| **3. Optional** | System yt-dlp **binary** subprocess for sites we have not ported yet (`ytdlp-subprocess` feature + settings) |
| **Never** | Ship or require Python; run yt-dlp’s Python source as our app |

“Copy yt-dlp” means: **port extractor behavior/ideas into Rust** and grow native coverage.  
Integration binary = temporary/extra coverage only.

---

## Division of labor

| Layer | Owner | Language |
|-------|--------|----------|
| Site extraction (primary) | **ferric-extract** native IEs | **Rust** |
| Format selector / info.json | ferric-extract | Rust |
| Optional coverage bridge | system **yt-dlp binary** via argv | external binary (not our code) |
| File download | **ferricd** | Rust |
| UX | ferric-ui / extension | Rust + TS |

---

## Default behavior

1. **Native pure-Rust IE** (REGISTRY + generic HTML/JSON scrape).
2. **yt-dlp binary** only if user enables it:
   - `ytdlp_max_compat=1` → always force binary extract  
   - `ytdlp_enable_fallback=1` → use binary when native has no usable formats  
   - or API `force_ytdlp: true`
3. Download always with **Ferric** (ferricd), not Python.

Feature `ytdlp-subprocess` can stay **compiled in** for the opt-in path; product still works with zero yt-dlp installed (pure Rust only).

---

## Settings keys

| Key | Meaning |
|-----|---------|
| `ytdlp_path` | Absolute path to optional yt-dlp **binary** |
| `ytdlp_cookies` | Absolute Netscape cookies.txt for that binary |
| `ytdlp_max_compat` | Force binary extract |
| `ytdlp_enable_fallback` | Allow binary when native fails / thin |

---

## Safety (subprocess path)

- No shell; argv only  
- No Python interpreter invoked by us  
- http(s) only, binary name allowlist, timeout, stdout cap  
- Never log cookies  

---

## Continue pure-Rust work

See **`docs/YOUTUBE-DLP-RUST-PORT.md`** for IE inventory and next ports.
