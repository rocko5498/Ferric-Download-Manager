-- FQT-102: RSS auto-download rules

CREATE TABLE IF NOT EXISTS rss_rules (
    id TEXT NOT NULL PRIMARY KEY,
    feed_id TEXT NOT NULL,
    name TEXT NOT NULL,
    match_pattern TEXT NOT NULL,
    action TEXT NOT NULL DEFAULT 'download',
    save_dir TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (feed_id) REFERENCES rss_feeds(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_rss_rules_feed ON rss_rules(feed_id);
