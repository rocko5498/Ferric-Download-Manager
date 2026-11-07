-- Migration 015: Add columns for HLS/DASH audio/subtitle track merging
ALTER TABLE downloads ADD COLUMN audio_url TEXT;
ALTER TABLE downloads ADD COLUMN subtitle_url TEXT;
ALTER TABLE downloads ADD COLUMN subtitle_lang TEXT;
