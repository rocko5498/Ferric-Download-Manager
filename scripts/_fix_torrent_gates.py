"""One-shot: unlock torrent start under system VPN (Cloudflare One).
Sets torrent_proxy_required=0, clears kill switch, clears per-torrent proxy_required.
Does NOT enable TCP listen (stays outbound-only).
"""
import json
import sqlite3
import sys
from pathlib import Path

db = Path.home() / "AppData/Local/ferric/ferric/data/ferric.db"
if not db.exists():
    print(f"missing {db}", file=sys.stderr)
    sys.exit(1)

c = sqlite3.connect(str(db))
c.row_factory = sqlite3.Row

print("BEFORE settings:")
for r in c.execute(
    "SELECT key, value FROM settings WHERE key LIKE '%torrent%' OR key LIKE '%kill%' OR key LIKE '%proxy%'"
):
    print(f"  {r['key']}={r['value']!r}")

print("BEFORE torrents:")
for r in c.execute(
    "SELECT id, name, status, proxy_required, engine FROM torrents"
):
    print(f"  {r['id'][:8]}… {r['status']} proxy_req={r['proxy_required']} name={r['name'][:60]!r}")

# System Cloudflare One / WARP is the network path — app SOCKS not required.
for key, val in [
    ("torrent_proxy_required", "0"),
    ("torrent_kill_switch_active", "0"),
    ("torrent_tcp_listen_enabled", "0"),  # keep outbound-only
    ("torrent_engine_enabled", "1"),
]:
    c.execute(
        "INSERT INTO settings(key, value) VALUES(?, ?) "
        "ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        (key, val),
    )

# Unlock staged torrents that inherited locked proxy_required from global gate.
c.execute("UPDATE torrents SET proxy_required = 0")
# Drop proxy_required from locked fields JSON if present
rows = list(c.execute("SELECT id, policy_locked_fields FROM torrents"))
for r in rows:
    raw = r["policy_locked_fields"] or "[]"
    try:
        fields = json.loads(raw)
    except json.JSONDecodeError:
        fields = []
    if isinstance(fields, list) and "proxy_required" in fields:
        fields = [f for f in fields if f != "proxy_required"]
        c.execute(
            "UPDATE torrents SET policy_locked_fields = ? WHERE id = ?",
            (json.dumps(fields), r["id"]),
        )

c.commit()

print("\nAFTER settings:")
for r in c.execute(
    "SELECT key, value FROM settings WHERE key IN "
    "('torrent_proxy_required','torrent_kill_switch_active',"
    "'torrent_tcp_listen_enabled','torrent_engine_enabled','torrent_proxy_url')"
):
    print(f"  {r['key']}={r['value']!r}")

print("AFTER torrents:")
for r in c.execute(
    "SELECT id, name, status, proxy_required, policy_locked_fields FROM torrents"
):
    print(
        f"  {r['id'][:8]}… {r['status']} proxy_req={r['proxy_required']} "
        f"locked={r['policy_locked_fields']}"
    )
print("done")
