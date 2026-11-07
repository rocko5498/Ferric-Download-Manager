-- Migration 017: Add column for per-download proxy URL
ALTER TABLE downloads ADD COLUMN proxy_url TEXT;
