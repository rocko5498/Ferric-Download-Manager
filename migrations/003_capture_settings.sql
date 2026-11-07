-- Migration 003: capture policy settings
-- Extension-side capture policy is now stored in the daemon settings table so
-- the desktop app and the extension share one source of truth.

INSERT OR IGNORE INTO settings VALUES ('capture_enabled',             '1');
INSERT OR IGNORE INTO settings VALUES ('capture_extensions',          '["3g2","3gp","7z","aac","aif","aiff","ape","apk","appimage","arj","avi","bin","bz2","cab","cbr","cbz","deb","dmg","doc","docx","exe","flac","flv","gz","gzip","iso","jar","m4a","m4v","mkv","mov","mp3","mp4","mpeg","mpg","msi","msix","msu","ogg","opus","pdf","pkg","ppt","pptx","rar","rpm","rtf","tar","tbz2","tgz","torrent","ts","txz","wav","webm","wim","wma","wmv","xls","xlsx","xz","zip","zipx"]');
INSERT OR IGNORE INTO settings VALUES ('capture_mime_prefixes',       '["application/","video/","audio/"]');
INSERT OR IGNORE INTO settings VALUES ('capture_exclude_urls',        '["*://*.gstatic.com/*/sounds/*","*://*.gvt1.com/edgedl/widevine-cdm/*","*://*.download.windowsupdate.com/*","*://download.windowsupdate.com/*","*://*.update.microsoft.com/*","*://*.facebook.com/*/sounds/*","*://*.fbcdn.net/*/sounds/*","*://*.whatsapp.com/*/res/*.mp3","*://*.cloudfront.net/game/*/res/*","*://*.akamaihd.net/*.mp3"]');
INSERT OR IGNORE INTO settings VALUES ('capture_panel_exclude_hosts', '["*.gstatic.com","media.tenor.co","media*.giphy.com"]');
INSERT OR IGNORE INTO settings VALUES ('capture_manifests',           '1');
INSERT OR IGNORE INTO settings VALUES ('capture_post_forms',          '1');
