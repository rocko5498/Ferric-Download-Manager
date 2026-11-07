-- Peer ban list for metadata anomaly detection (Task 055)
CREATE TABLE IF NOT EXISTS banned_peers (
    id          TEXT NOT NULL PRIMARY KEY,
    torrent_id  TEXT NOT NULL REFERENCES torrents(id) ON DELETE CASCADE,
    addr_hash   TEXT NOT NULL,
    reason      TEXT NOT NULL,
    banned_at   INTEGER NOT NULL,
    expires_at  INTEGER
);

CREATE INDEX IF NOT EXISTS idx_banned_peers_torrent ON banned_peers(torrent_id);
CREATE INDEX IF NOT EXISTS idx_banned_peers_addr ON banned_peers(addr_hash);
