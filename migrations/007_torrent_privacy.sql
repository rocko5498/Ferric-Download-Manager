-- Mako migration 007: torrent privacy gate fields

ALTER TABLE torrents ADD COLUMN is_private           INTEGER NOT NULL DEFAULT 0;
ALTER TABLE torrents ADD COLUMN policy_locked_fields TEXT    NOT NULL DEFAULT '';
ALTER TABLE torrents ADD COLUMN peer_id_seed         TEXT    NOT NULL DEFAULT '';
