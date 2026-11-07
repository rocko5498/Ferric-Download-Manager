-- Browser captures waiting for the Download File Info dialog. Their credentials live in memory
-- only, so rows still listed here after a restart can never be confirmed and are dropped at startup.
CREATE TABLE IF NOT EXISTS held_captures (id TEXT NOT NULL PRIMARY KEY);
