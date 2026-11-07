-- IDM comes with a "Synchronization queue" next to the main queue. Ours starts switched off (disabled):
-- files in it are re-checked every hour once it is enabled (Edit Queue).
INSERT OR IGNORE INTO queues (id, name, max_concurrent, enabled, priority, sync_interval_min)
VALUES ('00000000-0000-0000-0000-000000000002', 'Synchronization queue', 2, 0, -1, 60);
