-- Mako migration 005: persisted media merge jobs

CREATE TABLE IF NOT EXISTS merge_jobs (
    id           TEXT NOT NULL PRIMARY KEY,
    video_path   TEXT NOT NULL,
    audio_path   TEXT NOT NULL,
    output_path  TEXT NOT NULL,
    state        TEXT NOT NULL,
    message      TEXT,
    created_at   INTEGER NOT NULL,
    completed_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_merge_jobs_created ON merge_jobs(created_at DESC);
