#!/usr/bin/env python3
"""Browser-capture journeys: Chromium + the real extension + ferric-host + a real ferricd.

usage: capture.py FERRICD FERRIC_HOST EXTENSION_BUILD_DIR NODE_DIR CHROME_BINARY
  NODE_DIR is a folder where `npm i puppeteer-core` has been run. No display needed (new headless mode).
Each journey clicks a download link in the browser and checks what Ferric did with it.
"""
import base64, hashlib, json, os, shutil, socket, subprocess, sys, tempfile, threading, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import journeys as j


def extension_id(manifest_key_b64):
    digest = hashlib.sha256(base64.b64decode(manifest_key_b64)).hexdigest()[:32]
    return "".join(chr(ord("a") + int(c, 16)) for c in digest)


def main():
    ferricd, host, ext, pw, chrome_bin = sys.argv[1:6]
    manifest = json.load(open(os.path.join(ext, "manifest.json"), encoding="utf-8-sig"))
    ext_id = extension_id(manifest["key"])
    root = tempfile.mkdtemp(prefix="ferric-capture-")
    srv = j.Server(("0.0.0.0", 0), j.Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.connect(("10.255.255.255", 1)); lan = s.getsockname()[0]; s.close()
    base = f"http://{lan}:{srv.server_address[1]}"

    dm = j.Daemon(ferricd, root)
    shutil.copy2(host, os.path.join(dm.dir, "ferric-host"))   # beside the daemon: same portable data dir
    dm.start(); dm.call("POST", "/settings", {"allow_lan_downloads": "1"})

    user_data = os.path.join(root, "chromium-profile"); os.makedirs(os.path.join(user_data, "NativeMessagingHosts"))
    json.dump({"name": "com.ferric.host", "description": "Ferric", "path": os.path.join(dm.dir, "ferric-host"), "type": "stdio",
               "allowed_origins": [f"chrome-extension://{ext_id}/"]},
              open(os.path.join(user_data, "NativeMessagingHosts", "com.ferric.host.json"), "w"))
    browser_dl = os.path.join(root, "browser-downloads"); os.makedirs(browser_dl)

    def click(link, wait_ms=15000):
        env = dict(os.environ, WAIT_MS=str(wait_ms))
        shutil.copy2(os.path.join(os.path.dirname(os.path.abspath(__file__)), "capture.mjs"), os.path.join(pw, "capture.mjs"))  # ESM resolves the module next to the script
        # Output goes to a file, not a pipe: the native host Chrome starts can outlive the browser and would keep a pipe open.
        out = os.path.join(root, "browser-run.log")
        with open(out, "w") as f:
            proc = subprocess.Popen(["node", os.path.join(pw, "capture.mjs"), chrome_bin, ext, user_data, browser_dl, base + "/page.html", link],
                                    cwd=pw, env=env, stdout=f, stderr=subprocess.STDOUT)
            try:
                rc = proc.wait(timeout=90)
            except subprocess.TimeoutExpired:
                proc.kill(); rc = -9
        text = open(out).read()
        if rc != 0:
            raise RuntimeError("browser run failed (exit %s): %s" % (rc, text[:900] + " ... " + text[-300:]))
        if os.environ.get("FERRIC_E2E_VERBOSE"): print(text[-1500:])
        return text

    def ferric_downloads():
        return dm.call("GET", "/downloads")

    def files_in(d):
        return [f for f in os.listdir(d) if not f.endswith((".crdownload", ".tmp"))]  # files the browser kept itself

    failed = 0

    flt = sys.argv[sys.argv.index("-k") + 1] if "-k" in sys.argv else ""

    def run(name, fn):
        nonlocal failed
        if flt not in name: return
        t = time.time()
        try:
            fn(); print(f"PASS {name:28} {time.time() - t:5.1f}s")
        except Exception as e:
            failed += 1; print(f"FAIL {name:28} {type(e).__name__}: {e}")

    def captured(link, expect_name=None):
        before = {d["id"] for d in ferric_downloads()}
        click(link)
        time.sleep(2)
        new = [d for d in ferric_downloads() if d["id"] not in before]
        if os.environ.get("FERRIC_E2E_VERBOSE"): print("  Ferric now holds", [(d["filename"], d["status"]) for d in ferric_downloads()])
        assert new, f"Ferric did not receive the {link} download"
        d = dm.wait(new[0]["id"], {"completed", "error"}, 60)
        assert d["status"] == "completed", d.get("error_msg")
        assert j.sha_of(j.final_path(d)) == j.SHA, "captured file differs from the source"
        assert not files_in(browser_dl), f"the browser also saved its own copy: {files_in(browser_dl)}"
        if expect_name: assert d["filename"] == expect_name, d["filename"]

    run("capture_attachment", lambda: captured("attach", "attached.bin"))
    run("capture_redirect_link", lambda: captured("redirect"))
    run("capture_plain_link", lambda: captured("plain"))

    def daemon_down_falls_back():
        dm.kill(); time.sleep(1)
        click("plain", 25000)
        # The native host may start Ferric again (that is its job), so the file can end up in either place;
        # what must never happen is that neither the browser nor Ferric has it.
        kept = files_in(browser_dl)
        if kept:
            assert j.sha_of(os.path.join(browser_dl, kept[0])) == j.SHA, "browser copy is corrupt"
            for f in kept: os.remove(os.path.join(browser_dl, f))
            return
        assert dm.attach(5), "no browser copy, and Ferric was not started again: the download was lost"
        done = [d for d in ferric_downloads() if d["status"] == "completed" and j.sha_of(j.final_path(d)) == j.SHA]
        assert done, "no browser copy, and the restarted Ferric has no finished download: it was lost"
    run("daemon_down_browser_fallback", daemon_down_falls_back)

    dm.kill()
    print(f"{failed} failed")
    if failed or os.environ.get("FERRIC_E2E_KEEP"): print("logs kept in", root)
    else: shutil.rmtree(root, ignore_errors=True)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
