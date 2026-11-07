-- Optional yt-dlp *binary* bridge (product remains pure-Rust extract by default)
INSERT OR IGNORE INTO settings VALUES ('ytdlp_path', '');
INSERT OR IGNORE INTO settings VALUES ('ytdlp_cookies', '');
-- max_compat=1: always force standalone yt-dlp binary for extract
INSERT OR IGNORE INTO settings VALUES ('ytdlp_max_compat', '0');
-- enable_fallback=1: pure Rust first, then optional binary if native thin/fails
INSERT OR IGNORE INTO settings VALUES ('ytdlp_enable_fallback', '0');
