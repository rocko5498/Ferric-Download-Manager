import sqlite3
from pathlib import Path

db = Path.home() / "AppData/Local/ferric/ferric/data/ferric.db"
print("db", db, "exists", db.exists())
c = sqlite3.connect(db)
tables = [r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall()]
print("tables", tables)
if "torrents" in tables:
    rows = c.execute(
        "SELECT id, name, status, source_kind, substr(source,1,100) FROM torrents ORDER BY rowid DESC LIMIT 15"
    ).fetchall()
    print("torrent_count", len(rows))
    for r in rows:
        print(r)
else:
    print("no torrents table")
