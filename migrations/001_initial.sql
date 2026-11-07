-- Mako migration 001: initial schema
-- SQLite connection PRAGMAs are applied in db::init(), because sqlx 0.8
-- executes SQLite migrations inside a transaction.

CREATE TABLE IF NOT EXISTS downloads (
    id            TEXT NOT NULL PRIMARY KEY,
    url           TEXT NOT NULL,
    final_url     TEXT,
    filename      TEXT NOT NULL,
    save_path     TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'queued',
    total_size    INTEGER,
    downloaded    INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL,
    started_at    INTEGER,
    completed_at  INTEGER,
    error_msg     TEXT,
    error_count   INTEGER NOT NULL DEFAULT 0,
    referrer      TEXT,
    mime_type     TEXT,
    etag          TEXT,
    last_modified TEXT,
    category      TEXT NOT NULL DEFAULT 'Other',
    queue_id      TEXT,
    priority      INTEGER NOT NULL DEFAULT 0,
    checksum      TEXT,
    checksum_ok   INTEGER
);

CREATE TABLE IF NOT EXISTS chunks (
    id           TEXT NOT NULL PRIMARY KEY,
    download_id  TEXT NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
    chunk_index  INTEGER NOT NULL,
    byte_start   INTEGER NOT NULL,
    byte_end     INTEGER NOT NULL,
    downloaded   INTEGER NOT NULL DEFAULT 0,
    status       TEXT NOT NULL DEFAULT 'pending',
    error_count  INTEGER NOT NULL DEFAULT 0,
    UNIQUE(download_id, chunk_index)
);

CREATE TABLE IF NOT EXISTS cookies (
    id          TEXT NOT NULL PRIMARY KEY,
    download_id TEXT NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
    data_enc    BLOB NOT NULL,
    nonce       BLOB NOT NULL,
    domain      TEXT,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS site_rules (
    id        TEXT NOT NULL PRIMARY KEY,
    pattern   TEXT NOT NULL,
    rule_type TEXT NOT NULL,
    value     TEXT,
    priority  INTEGER NOT NULL DEFAULT 0
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_downloads_status    ON downloads(status);
CREATE INDEX IF NOT EXISTS idx_downloads_queue     ON downloads(queue_id, priority DESC);
CREATE INDEX IF NOT EXISTS idx_chunks_download     ON chunks(download_id, status);

-- Default settings
INSERT OR IGNORE INTO settings VALUES ('max_connections_global',   '128');
INSERT OR IGNORE INTO settings VALUES ('max_connections_per_host', '32');
INSERT OR IGNORE INTO settings VALUES ('bandwidth_limit_bps',      '0');
INSERT OR IGNORE INTO settings VALUES ('default_save_dir',         '');
INSERT OR IGNORE INTO settings VALUES ('min_capture_size',         '1048576');
INSERT OR IGNORE INTO settings VALUES ('auto_start',               '1');
INSERT OR IGNORE INTO settings VALUES ('theme',                    'system');
