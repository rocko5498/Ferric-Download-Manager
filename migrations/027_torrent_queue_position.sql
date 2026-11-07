-- Mako migration 027: torrent queue position (FQT-076/077)
-- api.rs::set_torrent_queue_position / list_torrents already assume this column
-- exists (queried since before this migration was written) -- 0 = bottom,
-- higher = higher priority, matches the up/down/top/bottom semantics in
-- set_torrent_queue_position.

ALTER TABLE torrents ADD COLUMN queue_position INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_torrents_queue_position ON torrents(queue_position DESC);
