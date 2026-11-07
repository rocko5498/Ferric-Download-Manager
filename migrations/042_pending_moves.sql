-- A download that started while its info dialog was open and was then given another name or folder:
-- the file is moved there when the transfer ends. Kept here so a daemon restart does not lose the choice.
CREATE TABLE pending_moves (
    id     TEXT PRIMARY KEY,
    target TEXT NOT NULL,
    name   TEXT NOT NULL
);
