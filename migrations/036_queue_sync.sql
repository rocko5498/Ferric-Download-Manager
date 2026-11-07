-- IDM's "Periodic synchronization": every N minutes re-check the completed files of the queue and
-- download again the ones that changed on the server. NULL = ordinary one-time queue.
ALTER TABLE queues ADD COLUMN sync_interval_min INTEGER;
