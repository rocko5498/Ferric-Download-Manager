-- IDM takes over only the file types on its list. The seeded "application/" mime prefix captured any
-- binary (a .dat, a game file); keep video/ and audio/, and leave application/* to the browser.
-- Only touched when the user never changed the seeded value.
UPDATE settings SET value = '["video/","audio/"]'
WHERE key = 'capture_mime_prefixes' AND value = '["application/","video/","audio/"]';
-- POST form capture was removed; the row has had no effect since.
DELETE FROM settings WHERE key = 'capture_post_forms';
