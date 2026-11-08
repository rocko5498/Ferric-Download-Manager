#!/usr/bin/env python3
"""End-to-end download journeys against a real ferricd (isolated data dir, local test server).

usage: journeys.py PATH_TO_FERRICD [-k name_filter]
Needs only the Python standard library. Exit code 0 = every journey passed.
"""
import hashlib, http.server, json, os, shutil, signal, socket, socketserver, subprocess, sys, tempfile, threading, time, urllib.request

FILE_SIZE = 24 * 1024 * 1024
DATA = os.urandom(FILE_SIZE)
SHA = hashlib.sha256(DATA).hexdigest()
if os.environ.get("E2E_SELFTEST_CORRUPT"):  # proves the checks can fail: serve one wrong byte
    DATA = DATA[:FILE_SIZE // 2] + bytes([DATA[FILE_SIZE // 2] ^ 1]) + DATA[FILE_SIZE // 2 + 1:]


class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True
    delay = 0.0          # seconds per 64 KB chunk
    drop_after = None    # bytes after which the first request is cut (flaky server)
    ranges_seen = []
    hits = 0


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def do_HEAD(self):
        self.do_GET(head=True)

    def do_GET(self, head=False):
        srv = self.server
        srv.hits += 1
        path = self.path.split("?")[0]
        if path == "/redirect":
            self.send_response(302); self.send_header("Location", "/file.bin"); self.send_header("Content-Length", "0"); self.end_headers(); return
        if path == "/missing":
            body = b"not here"
            self.send_response(404); self.send_header("Content-Length", str(len(body))); self.end_headers()
            if not head: self.wfile.write(body)
            return
        if path == "/page.bin":  # a web page pretending to be a file
            body = b"<!doctype html><html><body>Please log in</body></html>"
            self.send_response(200); self.send_header("Content-Type", "text/html"); self.send_header("Content-Length", str(len(body))); self.end_headers()
            if not head: self.wfile.write(body)
            return
        if path != "/file.bin":
            self.send_response(404); self.send_header("Content-Length", "0"); self.end_headers(); return
        start, end = 0, FILE_SIZE - 1
        rng = self.headers.get("Range")
        if rng and rng.startswith("bytes="):
            a, _, b = rng[6:].partition("-")
            start = int(a) if a else 0
            end = int(b) if b else FILE_SIZE - 1
            srv.ranges_seen.append(start)
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{end}/{FILE_SIZE}")
        else:
            self.send_response(200)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("ETag", '"ferric-test"')
        self.end_headers()
        if head:
            return
        sent = 0
        pos = start
        try:
            while pos <= end:
                n = min(65536, end - pos + 1)
                self.wfile.write(DATA[pos:pos + n]); pos += n; sent += n
                if srv.delay: time.sleep(srv.delay)
                if srv.drop_after is not None and sent >= srv.drop_after:
                    srv.drop_after = None  # only the first request is cut
                    self.connection.shutdown(2); return
        except (BrokenPipeError, ConnectionResetError):
            pass


class Daemon:
    def __init__(self, exe, root):
        self.root = root
        self.dir = os.path.join(root, "bin"); os.makedirs(self.dir)
        self.exe = os.path.join(self.dir, os.path.basename(exe))
        shutil.copy2(exe, self.exe)
        open(os.path.join(self.dir, "portable"), "w").close()  # data dir = bin/data
        self.dl = os.path.join(root, "dl"); os.makedirs(self.dl)
        self.proc = None

    def start(self):
        self.restarted = getattr(self, "restarted", False)
        self.log = open(os.path.join(self.root, "ferricd.log"), "ab")
        self.proc = subprocess.Popen([self.exe], stdout=self.log, stderr=self.log, cwd=self.dir)
        lock = os.path.join(self.dir, "data", "ferric.lock")
        for _ in range(100):
            time.sleep(0.2)
            if os.path.exists(lock):
                lines = open(lock).read().split()
                if len(lines) >= 2:
                    self.port, self.token = int(lines[0]), lines[1]
                    try:
                        self.call("GET", "/health")
                        if self.restarted: self.call("POST", "/settings", {"allow_lan_downloads": "1"})
                        self.restarted = True
                        return
                    except Exception:
                        pass
        raise RuntimeError("daemon did not start; see " + self.log.name)

    def kill(self):
        if self.proc:
            self.proc.send_signal(signal.SIGKILL); self.proc.wait(); self.proc = None
        lock = os.path.join(self.dir, "data", "ferric.lock")
        if os.path.exists(lock): os.remove(lock)

    def call(self, method, path, body=None):
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}{path}", method=method,
                                     data=json.dumps(body).encode() if body is not None else None,
                                     headers={"X-Ferric-Auth": getattr(self, "token", ""), "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=10) as r:
            raw = r.read()
            return json.loads(raw) if raw else None

    def add(self, url, **kw):
        return self.call("POST", "/downloads", {"url": url, "save_dir": self.dl, **kw})

    def get(self, id):
        return self.call("GET", f"/downloads/{id}")

    def wait(self, id, states, timeout=90):
        end = time.time() + timeout
        while time.time() < end:
            d = self.get(id)
            if d["status"] in states:
                return d
            time.sleep(0.4)
        raise AssertionError(f"timeout waiting for {states}; last={self.get(id)['status']}")


def sha_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""): h.update(b)
    return h.hexdigest()


def final_path(d):
    p = d["save_path"]
    return p if os.path.isfile(p) else os.path.join(p, d["filename"])


# ---------------------------------------------------------------- journeys
def j_basic(dm, srv, base):
    d = dm.add(base + "/file.bin"); d = dm.wait(d["id"], {"completed", "error"})
    assert d["status"] == "completed", d.get("error_msg")
    assert sha_of(final_path(d)) == SHA, "content differs from the source"


def j_redirect(dm, srv, base):
    d = dm.add(base + "/redirect"); d = dm.wait(d["id"], {"completed", "error"})
    assert d["status"] == "completed", d.get("error_msg")
    assert sha_of(final_path(d)) == SHA


def j_pause_resume(dm, srv, base):
    srv.delay = 0.02
    d = dm.add(base + "/file.bin")
    for _ in range(100):
        if dm.get(d["id"])["downloaded"] > 1_000_000: break
        time.sleep(0.2)
    dm.call("POST", f"/downloads/{d['id']}/pause"); time.sleep(1.5)
    p = dm.wait(d["id"], {"paused"}, 20)
    a = p["downloaded"]; time.sleep(2); b = dm.get(d["id"])["downloaded"]
    assert b == a, f"bytes kept arriving while paused ({a} -> {b})"
    assert 0 < a < FILE_SIZE
    srv.delay = 0.0
    dm.call("POST", f"/downloads/{d['id']}/resume")
    d = dm.wait(d["id"], {"completed", "error"})
    assert d["status"] == "completed", d.get("error_msg")
    assert sha_of(final_path(d)) == SHA, "content differs after pause/resume"


def j_crash_resume(dm, srv, base):
    srv.delay = 0.03
    srv.ranges_seen.clear()
    d = dm.add(base + "/file.bin")
    for _ in range(100):
        if dm.get(d["id"])["downloaded"] > 2_000_000: break
        time.sleep(0.2)
    dm.kill(); srv.delay = 0.0
    dm.start()
    got = dm.get(d["id"])
    if got["status"] in ("paused", "error", "queued"):
        dm.call("POST", f"/downloads/{d['id']}/resume")
    d = dm.wait(d["id"], {"completed", "error"}, 120)
    assert d["status"] == "completed", d.get("error_msg")
    assert sha_of(final_path(d)) == SHA, "content differs after a daemon crash"
    assert any(r > 0 for r in srv.ranges_seen), "restart re-downloaded from zero instead of resuming"


def j_flaky_server(dm, srv, base):
    srv.drop_after = 5_000_000
    d = dm.add(base + "/file.bin"); d = dm.wait(d["id"], {"completed", "error"}, 120)
    assert d["status"] == "completed", d.get("error_msg")
    assert sha_of(final_path(d)) == SHA, "content differs after a dropped connection"


def j_not_found(dm, srv, base):
    d = dm.add(base + "/missing"); d = dm.wait(d["id"], {"error", "completed"})
    assert d["status"] == "error" and d.get("error_msg"), d
    assert "404" in d["error_msg"] or "not" in d["error_msg"].lower(), d["error_msg"]


def j_web_page(dm, srv, base):
    d = dm.add(base + "/page.bin"); d = dm.wait(d["id"], {"error", "completed"})
    assert d["status"] == "error", "an HTML login page was accepted as the file"


def j_checksum_ok(dm, srv, base):
    d = dm.add(base + "/file.bin", checksum="sha256:" + SHA); d = dm.wait(d["id"], {"completed", "error"})
    assert d["status"] == "completed" and d.get("checksum_ok") is True, d


def j_checksum_bad(dm, srv, base):
    d = dm.add(base + "/file.bin", checksum="sha256:" + "0" * 64); d = dm.wait(d["id"], {"completed", "error"})
    assert d["status"] == "error" or d.get("checksum_ok") is False, "a wrong checksum was accepted"


def j_cancel_removes(dm, srv, base):
    srv.delay = 0.03
    d = dm.add(base + "/file.bin"); time.sleep(1.5)
    dm.call("POST", f"/downloads/{d['id']}/cancel"); srv.delay = 0.0
    d = dm.wait(d["id"], {"cancelled", "error", "paused"}, 20)
    assert d["status"] == "cancelled", d["status"]


JOURNEYS = [j_basic, j_redirect, j_pause_resume, j_crash_resume, j_flaky_server, j_not_found,
            j_web_page, j_checksum_ok, j_checksum_bad, j_cancel_removes]


def main():
    exe = sys.argv[1]
    flt = sys.argv[sys.argv.index("-k") + 1] if "-k" in sys.argv else ""
    root = tempfile.mkdtemp(prefix="ferric-e2e-")
    srv = Server(("0.0.0.0", 0), Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    # The daemon refuses loopback (SSRF guard), so serve on this machine's private LAN address.
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.connect(("10.255.255.255", 1)); lan = s.getsockname()[0]; s.close()
    base = f"http://{lan}:{srv.server_address[1]}"
    dm = Daemon(exe, root)
    dm.start()
    dm.call("POST", "/settings", {"allow_lan_downloads": "1"})
    failed = 0
    for j in JOURNEYS:
        if flt not in j.__name__: continue
        srv.delay = 0.0; srv.drop_after = None
        t = time.time()
        try:
            j(dm, srv, base); print(f"PASS {j.__name__:20} {time.time() - t:5.1f}s")
        except Exception as e:
            failed += 1; print(f"FAIL {j.__name__:20} {type(e).__name__}: {e}")
            if dm.proc is None:
                dm.start()
    dm.kill()
    print(f"{len(JOURNEYS) - failed if not flt else '-'} passed, {failed} failed  (logs: {root})")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
