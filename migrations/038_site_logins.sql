-- IDM's Options -> Sites Logins: a user name and password per server (optionally server/path).
-- The password is stored encrypted with the Windows user's key (DPAPI), never in clear text.
CREATE TABLE site_logins (
    id           TEXT PRIMARY KEY,
    server       TEXT NOT NULL UNIQUE,
    username     TEXT NOT NULL,
    password_enc TEXT NOT NULL
);
