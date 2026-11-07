-- Days of the week a queue may run (IDM's scheduler Daily checkboxes). Bit 0 = Sunday ... bit 6 = Saturday.
ALTER TABLE queues ADD COLUMN sched_days INTEGER NOT NULL DEFAULT 127;
