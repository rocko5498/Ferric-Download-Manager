-- Mako migration 006: torrent staging and policy model

CREATE TABLE IF NOT EXISTS torrents (
    id                  TEXT NOT NULL PRIMARY KEY,
    info_hash           TEXT NOT NULL UNIQUE,
    name                TEXT NOT NULL,
    source              TEXT NOT NULL,
    source_kind         TEXT NOT NULL,
    save_root           TEXT,
    status              TEXT NOT NULL,
    total_size          INTEGER NOT NULL DEFAULT 0,
    downloaded          INTEGER NOT NULL DEFAULT 0,
    uploaded            INTEGER NOT NULL DEFAULT 0,
    ratio               REAL NOT NULL DEFAULT 0,
    dht_enabled         INTEGER NOT NULL DEFAULT 0,
    pex_enabled         INTEGER NOT NULL DEFAULT 0,
    lsd_enabled         INTEGER NOT NULL DEFAULT 0,
    seed_enabled        INTEGER NOT NULL DEFAULT 0,
    seed_ratio_limit    REAL,
    seed_time_limit_min INTEGER,
    upload_limit_bps    INTEGER,
    proxy_required      INTEGER NOT NULL DEFAULT 0,
    blocklist_enabled   INTEGER NOT NULL DEFAULT 0,
    engine              TEXT NOT NULL DEFAULT 'gated',
    created_at          INTEGER NOT NULL,
    completed_at        INTEGER
);

CREATE TABLE IF NOT EXISTS torrent_files (
    id          TEXT NOT NULL PRIMARY KEY,
    torrent_id  TEXT NOT NULL REFERENCES torrents(id) ON DELETE CASCADE,
    path        TEXT NOT NULL,
    size        INTEGER NOT NULL,
    selected    INTEGER NOT NULL DEFAULT 1,
    priority    INTEGER NOT NULL DEFAULT 0,
    downloaded  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS torrent_trackers (
    id          TEXT NOT NULL PRIMARY KEY,
    torrent_id  TEXT NOT NULL REFERENCES torrents(id) ON DELETE CASCADE,
    url         TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'staged',
    last_error  TEXT
);

CREATE TABLE IF NOT EXISTS torrent_peers (
    id          TEXT NOT NULL PRIMARY KEY,
    torrent_id  TEXT NOT NULL REFERENCES torrents(id) ON DELETE CASCADE,
    addr_hash   TEXT NOT NULL,
    client      TEXT,
    downloaded  INTEGER NOT NULL DEFAULT 0,
    uploaded    INTEGER NOT NULL DEFAULT 0,
    state       TEXT NOT NULL DEFAULT 'staged'
);

CREATE TABLE IF NOT EXISTS torrent_pieces (
    id           TEXT NOT NULL PRIMARY KEY,
    torrent_id   TEXT NOT NULL REFERENCES torrents(id) ON DELETE CASCADE,
    piece_index  INTEGER NOT NULL,
    size         INTEGER NOT NULL,
    status       TEXT NOT NULL DEFAULT 'missing',
    availability INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_torrents_created ON torrents(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_torrent_files_torrent ON torrent_files(torrent_id);
CREATE INDEX IF NOT EXISTS idx_torrent_trackers_torrent ON torrent_trackers(torrent_id);
CREATE INDEX IF NOT EXISTS idx_torrent_peers_torrent ON torrent_peers(torrent_id);
CREATE INDEX IF NOT EXISTS idx_torrent_pieces_torrent ON torrent_pieces(torrent_id, piece_index);
