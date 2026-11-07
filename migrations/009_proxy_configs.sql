CREATE TABLE IF NOT EXISTS proxy_configs (
    id           TEXT NOT NULL PRIMARY KEY,
    name         TEXT NOT NULL,
    proxy_type   TEXT NOT NULL,
    host         TEXT NOT NULL,
    port         INTEGER NOT NULL,
    username     TEXT,
    password_enc BLOB,
    nonce        BLOB,
    created_at   INTEGER NOT NULL
);

ALTER TABLE torrents ADD COLUMN proxy_id TEXT REFERENCES proxy_configs(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_torrents_proxy ON torrents(proxy_id);
