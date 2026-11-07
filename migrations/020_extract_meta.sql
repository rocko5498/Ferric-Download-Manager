-- Persist pure-Rust ExtractResult JSON for post-download sidecars (info.json, chapters, …)
CREATE TABLE IF NOT EXISTS download_extract_meta (
    download_id TEXT NOT NULL PRIMARY KEY REFERENCES downloads(id) ON DELETE CASCADE,
    meta_json   TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

INSERT OR IGNORE INTO settings VALUES ('sidecar_info_json', '1');
INSERT OR IGNORE INTO settings VALUES ('sidecar_description', '1');
INSERT OR IGNORE INTO settings VALUES ('sidecar_chapters', '1');
INSERT OR IGNORE INTO settings VALUES ('sidecar_embed_thumb', '0');
