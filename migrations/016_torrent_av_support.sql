-- Seed AV settings
INSERT OR IGNORE INTO settings VALUES ('av_enabled', '0');
INSERT OR IGNORE INTO settings VALUES ('av_path', '');
INSERT OR IGNORE INTO settings VALUES ('av_args', '');

-- Add AV columns to torrents table
ALTER TABLE torrents ADD COLUMN av_status TEXT;
ALTER TABLE torrents ADD COLUMN av_result TEXT;
