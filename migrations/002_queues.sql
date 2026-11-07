-- Mako migration 002: queues table

CREATE TABLE IF NOT EXISTS queues (
    id             TEXT NOT NULL PRIMARY KEY,
    name           TEXT NOT NULL UNIQUE,
    max_concurrent INTEGER NOT NULL DEFAULT 2,
    max_bandwidth  INTEGER,
    sched_start    INTEGER,
    sched_end      INTEGER,
    enabled        INTEGER NOT NULL DEFAULT 1,
    priority       INTEGER NOT NULL DEFAULT 0
);

-- Default queue
INSERT OR IGNORE INTO queues (id, name, max_concurrent, enabled, priority)
VALUES ('00000000-0000-0000-0000-000000000001', 'Default', 2, 1, 0);

-- Add FK column to downloads if not already present (SQLite ALTER TABLE limitation)
-- If you need to add foreign key enforcement, recreate the table in a later migration.
UPDATE settings SET value = '00000000-0000-0000-0000-000000000001'
WHERE key = 'default_queue_id';

INSERT OR IGNORE INTO settings VALUES ('default_queue_id', '00000000-0000-0000-0000-000000000001');
