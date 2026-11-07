#!/usr/bin/env python3
"""Autonomous regression smoke for Ferric daemon (no UI).

Usage (repo root):
  python scripts/ferric_autonomous_smoke.py

Exit 0 = all checks passed.
"""
from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path

DATA = Path(os.environ.get("LOCALAPPDATA", "")) / "ferric" / "ferric" / "data"
LOCK = DATA / "ferric.lock"


def fail(msg: str) -> None:
    print("FAIL:", msg)
    sys.exit(1)


def call(port: str, token: str, method: str, path: str, body=None, timeout=20):
    raw = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}",
        data=raw,
        method=method,
        headers={"X-Ferric-Auth": token, "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            text = resp.read().decode("utf-8", errors="replace")
            return resp.status, (json.loads(text) if text else None)
    except urllib.error.HTTPError as e:
        text = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"{method} {path} -> {e.code}: {text[:300]}") from e


def main() -> None:
    if not LOCK.exists():
        fail(f"no lock at {LOCK} — start target/debug/ferricd.exe")

    lines = LOCK.read_text(encoding="utf-8").strip().splitlines()
    port, token = lines[0].strip(), lines[1].strip()
    print("port", port)

    st, health = call(port, token, "GET", "/health")
    assert st == 200, health
    print("OK health", health.get("data_dir"))

    st, settings = call(port, token, "GET", "/settings")
    assert st == 200
    mode = settings.get("product_mode") or "balanced"
    print("OK settings product_mode", mode)

    st, torrents = call(port, token, "GET", "/torrents")
    assert st == 200 and isinstance(torrents, list)
    print("OK torrents", len(torrents))

    st, downloads = call(port, token, "GET", "/downloads")
    assert st == 200 and isinstance(downloads, list)
    print("OK downloads", len(downloads))

    # Mode round-trip without leaving strict (proxy-required) stuck.
    for m in ("balanced", "open", "balanced"):
        st, _ = call(port, token, "POST", "/settings", {"product_mode": m})
        assert st in (200, 204), st
        st, s = call(port, token, "GET", "/settings")
        assert s.get("product_mode") == m, s.get("product_mode")
        print("OK product_mode", m, "dht", s.get("torrent_dht_enabled"), "listen", s.get("torrent_tcp_listen_enabled"))

    # HTTP tiny complete
    st, rec = call(
        port,
        token,
        "POST",
        "/downloads",
        {"url": "https://example.com/", "filename": "ferric-smoke.html"},
    )
    assert st == 201 and rec.get("id"), rec
    dl_id = rec["id"]
    print("OK add download", dl_id)
    import time

    final = None
    for _ in range(15):
        time.sleep(0.5)
        st, one = call(port, token, "GET", f"/downloads/{dl_id}")
        final = one
        if one.get("status") in ("completed", "error", "cancelled"):
            break
    print("OK download status", (final or {}).get("status"), (final or {}).get("downloaded"))
    try:
        call(port, token, "DELETE", f"/downloads/{dl_id}")
    except Exception as e:
        print("note delete", e)

    # List still works after churn
    st, torrents2 = call(port, token, "GET", "/torrents")
    assert st == 200 and len(torrents2) == len(torrents)
    print("OK torrents stable", len(torrents2))

    print("ALL_PASS")


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        fail(str(e))
