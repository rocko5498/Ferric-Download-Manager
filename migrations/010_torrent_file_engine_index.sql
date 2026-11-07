-- Mako migration 010: rqbit file index handoff

ALTER TABLE torrent_files ADD COLUMN engine_index INTEGER;
