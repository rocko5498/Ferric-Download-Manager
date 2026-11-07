-- Mako migration 025: per-torrent tags

ALTER TABLE torrents ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';
