-- IDM's scheduler option "Start download on IDM startup": start this queue whenever the daemon starts.
ALTER TABLE queues ADD COLUMN start_on_startup INTEGER NOT NULL DEFAULT 0;
