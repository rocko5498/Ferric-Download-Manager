-- Migration 018: Alternate mirror URLs for multi-source downloads (JSON array)
ALTER TABLE downloads ADD COLUMN mirrors TEXT;
