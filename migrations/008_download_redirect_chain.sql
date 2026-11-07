-- Persist observed redirect endpoints for diagnostics.
-- The current downloader records original URL + final URL; the JSON array shape
-- leaves room for full intermediate hops if the transport layer exposes them.
ALTER TABLE downloads ADD COLUMN redirect_chain TEXT;
