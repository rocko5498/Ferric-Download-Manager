-- Mako migration 011: torrent anti-fingerprint policy gates

ALTER TABLE torrents ADD COLUMN mse_required INTEGER NOT NULL DEFAULT 0;
