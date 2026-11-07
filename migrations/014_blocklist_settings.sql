-- Migration 014: blocklist settings
INSERT OR IGNORE INTO settings VALUES ('blocklist_url', 'https://raw.githubusercontent.com/runet-blocklist/p2p-blocklist/master/blocklist.txt');
