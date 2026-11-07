-- Task 123: per-download network audit (never cookies)
CREATE TABLE IF NOT EXISTS download_audit (
    id           TEXT NOT NULL PRIMARY KEY,
    download_id  TEXT NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
    event_type   TEXT NOT NULL,
    detail_json  TEXT NOT NULL,
    created_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_download_audit_dl ON download_audit(download_id, created_at);

-- Task 115: user format presets (JSON array)
INSERT OR IGNORE INTO settings VALUES ('user_format_presets', '[]');
-- Task 108: preferred subtitle languages (comma-separated)
INSERT OR IGNORE INTO settings VALUES ('subtitle_langs', 'en');
INSERT OR IGNORE INTO settings VALUES ('subtitle_auto_download', '1');
INSERT OR IGNORE INTO settings VALUES ('subtitle_vtt_to_srt', '1');
-- Task 130: purge cookies when download completes
INSERT OR IGNORE INTO settings VALUES ('cookie_purge_on_complete', '1');
-- Task 131: API rate limit (requests per minute per token)
INSERT OR IGNORE INTO settings VALUES ('api_rate_limit_per_min', '600');
