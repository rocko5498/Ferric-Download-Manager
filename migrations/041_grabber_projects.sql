-- IDM's Site Grabber keeps projects (settings plus what was found) and templates (settings only).
CREATE TABLE grabber_projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    is_template INTEGER NOT NULL DEFAULT 0,
    config      TEXT NOT NULL,
    results     TEXT NOT NULL DEFAULT '[]',
    updated_at  INTEGER NOT NULL
);
