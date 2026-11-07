-- Tasks 124/132/180/183/184/185/188/189 + UX

CREATE TABLE IF NOT EXISTS network_audit_global (
    id TEXT NOT NULL PRIMARY KEY,
    event_type TEXT NOT NULL,
    host TEXT,
    detail_json TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_net_audit_created ON network_audit_global(created_at);

CREATE TABLE IF NOT EXISTS torrent_audit (
    id TEXT NOT NULL PRIMARY KEY,
    torrent_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    data_json TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_torrent_audit_tid ON torrent_audit(torrent_id, created_at);

CREATE TABLE IF NOT EXISTS webhooks (
    id TEXT NOT NULL PRIMARY KEY,
    url TEXT NOT NULL,
    events TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS rss_feeds (
    id TEXT NOT NULL PRIMARY KEY,
    url TEXT NOT NULL,
    poll_secs INTEGER NOT NULL DEFAULT 3600,
    last_poll_at INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS download_categories (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    pattern TEXT,
    save_subdir TEXT
);

-- Torrent privacy settings seeds
INSERT OR IGNORE INTO settings VALUES ('torrent_dht_enabled', '0');
INSERT OR IGNORE INTO settings VALUES ('torrent_lsd_enabled', '0');
INSERT OR IGNORE INTO settings VALUES ('torrent_pex_enabled', '0');
INSERT OR IGNORE INTO settings VALUES ('torrent_proxy_url', '');
INSERT OR IGNORE INTO settings VALUES ('torrent_proxy_required', '1');
INSERT OR IGNORE INTO settings VALUES ('torrent_proxy_warned_at', '');
INSERT OR IGNORE INTO settings VALUES ('torrent_blocklist_url', '');
INSERT OR IGNORE INTO settings VALUES ('torrent_blocklist_last_update', '0');
INSERT OR IGNORE INTO settings VALUES ('torrent_kill_switch_active', '0');
INSERT OR IGNORE INTO settings VALUES ('torrent_seeding_default', '0');
INSERT OR IGNORE INTO settings VALUES ('torrent_seed_ratio_limit', '1.0');
INSERT OR IGNORE INTO settings VALUES ('torrent_seed_time_limit_secs', '0');
INSERT OR IGNORE INTO settings VALUES ('audit_mode', '1');
INSERT OR IGNORE INTO settings VALUES ('audit_retention_days', '30');
INSERT OR IGNORE INTO settings VALUES ('webhook_enabled', '0');
INSERT OR IGNORE INTO settings VALUES ('ui_language', 'en');
INSERT OR IGNORE INTO settings VALUES ('dedupe_enabled', '1');
INSERT OR IGNORE INTO settings VALUES ('portable_mode', '0');
INSERT OR IGNORE INTO settings VALUES ('chunk_size_jitter_pct', '10');
INSERT OR IGNORE INTO settings VALUES ('geoip_mmdb_path', '');
INSERT OR IGNORE INTO settings VALUES ('onboarding_done', '0');
INSERT OR IGNORE INTO settings VALUES ('auto_update_channel', 'stable');
