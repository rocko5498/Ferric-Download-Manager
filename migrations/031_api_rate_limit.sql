-- The progress windows poll the local API several times a second each; the seeded 600/min cap was
-- reached with two windows open. Raise it only where the user never changed it.
UPDATE settings SET value = '3000' WHERE key = 'api_rate_limit_per_min' AND value = '600';
