-- Hosts used to be capped at the number of connections a download started with (priority 50, "learned
-- success"), which stopped later downloads from ramping up. Only real limits remain: user rules and
-- learned rate limits (priority 100).
DELETE FROM site_rules WHERE rule_type = 'max_connections' AND priority = 50;
