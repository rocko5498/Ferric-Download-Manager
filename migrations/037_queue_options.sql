-- IDM keeps these scheduler options per queue: retries per file, a file to open when the queue
-- finishes, and what to do then ('exit' / 'shutdown'). NULL = use the global preference.
ALTER TABLE queues ADD COLUMN max_retries INTEGER;
ALTER TABLE queues ADD COLUMN open_file TEXT;
ALTER TABLE queues ADD COLUMN when_done TEXT;
