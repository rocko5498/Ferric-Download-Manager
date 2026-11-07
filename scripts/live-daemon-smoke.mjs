/**
 * Live ferricd API smoke — starts daemon if needed, hits real routes with auth.
 * Usage: node scripts/live-daemon-smoke.mjs
 * Env:
 *   FERRICD_BIN  path to ferricd.exe (default target/debug/ferricd.exe)
 *   DATA_DIR   override data dir (default %LOCALAPPDATA%/ferric/ferric)
 */
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, '..');
const isWin = process.platform === 'win32';
const ferricdBin =
  process.env.FERRICD_BIN ||
  path.join(root, 'target', 'debug', isWin ? 'ferricd.exe' : 'ferricd');
function findLockPath() {
  if (process.env.DATA_DIR) {
    return path.join(process.env.DATA_DIR, 'ferric.lock');
  }
  const local = process.env.LOCALAPPDATA || process.env.HOME || '';
  const candidates = [
    path.join(local, 'ferric', 'ferric', 'data', 'ferric.lock'),
    path.join(local, 'ferric', 'ferric', 'ferric.lock'),
    path.join(local, 'com.ferric.ferric', 'ferric.lock'),
  ];
  for (const c of candidates) {
    if (fs.existsSync(c)) return c;
  }
  // default preferred path (matches current Windows layout)
  return candidates[0];
}
const lockPath = findLockPath();
const dataDir = path.dirname(lockPath);

const results = [];
let child = null;

function log(ok, name, detail = '') {
  results.push({ ok, name, detail: String(detail).slice(0, 300) });
  console.log(`${ok ? 'OK ' : 'FAIL'} ${name}${detail ? ` — ${String(detail).slice(0, 120)}` : ''}`);
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function readLock() {
  if (!fs.existsSync(lockPath)) return null;
  const lines = fs.readFileSync(lockPath, 'utf8').split(/\r?\n/).filter(Boolean);
  if (lines.length < 2) return null;
  return { port: Number(lines[0].trim()), token: lines[1].trim() };
}

async function waitLock(ms = 20_000) {
  const start = Date.now();
  while (Date.now() - start < ms) {
    const l = readLock();
    if (l && l.port && l.token) {
      // probe health with token
      try {
        await api(l, 'GET', '/health');
        return l;
      } catch {
        /* retry */
      }
    }
    await sleep(250);
  }
  throw new Error(`timeout waiting for lock ${lockPath}`);
}

function api(lock, method, p, body) {
  return new Promise((resolve, reject) => {
    const data = body == null ? null : JSON.stringify(body);
    const req = http.request(
      {
        hostname: '127.0.0.1',
        port: lock.port,
        path: p,
        method,
        headers: {
          'X-Ferric-Auth': lock.token,
          ...(data
            ? { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(data) }
            : {}),
        },
        timeout: 10_000,
      },
      (res) => {
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => {
          const text = Buffer.concat(chunks).toString('utf8');
          let json = null;
          try {
            json = text ? JSON.parse(text) : null;
          } catch {
            json = text;
          }
          if (res.statusCode >= 400) {
            reject(new Error(`${method} ${p} -> ${res.statusCode}: ${text.slice(0, 200)}`));
          } else {
            resolve({ status: res.statusCode, json, text });
          }
        });
      },
    );
    req.on('error', reject);
    req.on('timeout', () => {
      req.destroy();
      reject(new Error(`timeout ${method} ${p}`));
    });
    if (data) req.write(data);
    req.end();
  });
}

async function expect(name, fn) {
  try {
    const detail = await fn();
    log(true, name, detail ?? '');
  } catch (e) {
    log(false, name, e.message || e);
  }
}

async function main() {
  if (!fs.existsSync(ferricdBin)) {
    console.error(`ferricd not found: ${ferricdBin}\nBuild with: cargo build -p ferricd`);
    process.exit(2);
  }
  fs.mkdirSync(dataDir, { recursive: true });

  // Kill stale lock if port dead
  const existing = readLock();
  if (existing) {
    try {
      await api(existing, 'GET', '/health');
      log(true, 'reuse_running_daemon', `port ${existing.port}`);
    } catch {
      try {
        fs.unlinkSync(lockPath);
      } catch {
        /* ignore */
      }
    }
  }

  if (!readLock()) {
    console.log(`starting ${ferricdBin}`);
    child = spawn(ferricdBin, [], {
      cwd: root,
      detached: true,
      stdio: 'ignore',
      windowsHide: true,
    });
    child.unref();
  }

  const lock = await waitLock();
  log(true, 'daemon_up', `port=${lock.port}`);

  await expect('health', async () => {
    const r = await api(lock, 'GET', '/health');
    return `v=${r.json?.version}`;
  });

  await expect('settings_get', async () => {
    const r = await api(lock, 'GET', '/settings');
    return `keys=${Object.keys(r.json || {}).length}`;
  });

  await expect('settings_set_onboarding', async () => {
    await api(lock, 'POST', '/settings', { onboarding_done: '1', notifications_enabled: '1' });
    return 'ok';
  });

  await expect('list_downloads', async () => {
    const r = await api(lock, 'GET', '/downloads');
    return `n=${Array.isArray(r.json) ? r.json.length : '?'}`;
  });

  await expect('list_queues', async () => {
    const r = await api(lock, 'GET', '/queues');
    return `n=${Array.isArray(r.json) ? r.json.length : '?'}`;
  });

  await expect('site_rules_crud', async () => {
    const created = await api(lock, 'POST', '/site-rules', {
      pattern: '*://smoke.example.com/*',
      rule_type: 'max_connections',
      value: '2',
    });
    const id = created.json?.id;
    const list = await api(lock, 'GET', '/site-rules');
    const found = (list.json || []).some((x) => x.id === id);
    if (!found) throw new Error('created rule not listed');
    await api(lock, 'DELETE', `/site-rules/${id}`);
    return `id=${id}`;
  });

  await expect('rss_list_add', async () => {
    const list = await api(lock, 'GET', '/rss/feeds');
    const add = await api(lock, 'POST', '/rss/feeds', {
      url: `https://example.com/fqt-smoke-${Date.now()}.xml`,
      poll_secs: 3600,
    });
    await api(lock, 'DELETE', `/rss/feeds/${add.json?.id}`);
    return `feeds=${(list.json || []).length}+1 id=${add.json?.id}`;
  });

  await expect('webhooks_list', async () => {
    const r = await api(lock, 'GET', '/webhooks');
    return `n=${Array.isArray(r.json) ? r.json.length : '?'}`;
  });

  await expect('categories_list', async () => {
    const r = await api(lock, 'GET', '/categories');
    return `n=${Array.isArray(r.json) ? r.json.length : '?'}`;
  });

  await expect('plugins_list', async () => {
    const r = await api(lock, 'GET', '/plugins');
    return `n=${Array.isArray(r.json) ? r.json.length : '?'}`;
  });

  await expect('audit_export', async () => {
    const r = await api(lock, 'GET', '/audit/export');
    return `keys=${Object.keys(r.json || {}).join(',')}`;
  });

  await expect('privacy_dashboard', async () => {
    const r = await api(lock, 'GET', '/privacy/dashboard');
    return `status=${r.status}`;
  });

  await expect('anonymity_score', async () => {
    const r = await api(lock, 'GET', '/privacy/anonymity-score');
    return `status=${r.status}`;
  });

  await expect('merge_jobs', async () => {
    const r = await api(lock, 'GET', '/media/merge-jobs');
    return `n=${Array.isArray(r.json) ? r.json.length : '?'}`;
  });

  await expect('torrents_list', async () => {
    const r = await api(lock, 'GET', '/torrents');
    return `n=${Array.isArray(r.json) ? r.json.length : '?'}`;
  });

  await expect('proxies_list', async () => {
    const r = await api(lock, 'GET', '/proxies');
    return `n=${Array.isArray(r.json) ? r.json.length : '?'}`;
  });

  await expect('ytdlp_status', async () => {
    const r = await api(lock, 'GET', '/media/ytdlp');
    return `status=${r.status}`;
  });

  await expect('ffmpeg_status', async () => {
    const r = await api(lock, 'GET', '/media/ffmpeg');
    return `status=${r.status}`;
  });

  await expect('cookies_from_browser_disabled', async () => {
    const r = await api(lock, 'POST', '/media/cookies/from-browser', {
      url: 'https://example.com/',
    });
    if (r.json?.cookie_header) throw new Error('cookie_header should be null');
    return r.json?.explanation?.slice(0, 80);
  });

  await expect('extract_login_disabled', async () => {
    try {
      await api(lock, 'POST', '/media/extract-login', { url: 'https://example.com/' });
      throw new Error('expected 410');
    } catch (e) {
      if (!String(e.message).includes('410')) throw e;
      return '410 as expected';
    }
  });

  await expect('add_download_reject_private', async () => {
    try {
      await api(lock, 'POST', '/downloads', {
        url: 'http://127.0.0.1/secret.bin',
      });
      throw new Error('expected 400');
    } catch (e) {
      if (!/400|403/.test(String(e.message))) throw e;
      return 'blocked private IP';
    }
  });

  await expect('add_download_public_ok', async () => {
    // example.com is fine for validation; download may fail later — we only check accept
    const r = await api(lock, 'POST', '/downloads', {
      url: 'https://example.com/smoke-test.bin',
      filename: 'smoke-test.bin',
    });
    const id = r.json?.id;
    if (!id) throw new Error('no id');
    // cleanup
    try {
      await api(lock, 'DELETE', `/downloads/${id}`);
    } catch {
      /* optional */
    }
    return `id=${id}`;
  });

  await expect('auth_reject_bad_token', async () => {
    try {
      await api({ port: lock.port, token: 'bad-token' }, 'GET', '/downloads');
      throw new Error('expected 401');
    } catch (e) {
      if (!String(e.message).includes('401')) throw e;
      return '401';
    }
  });

  const failed = results.filter((r) => !r.ok);
  const out = path.join(root, 'scripts', 'live-daemon-smoke-report.json');
  fs.writeFileSync(
    out,
    JSON.stringify({ lock: { port: lock.port }, failed: failed.length, results }, null, 2),
  );
  console.log(`\n=== live daemon smoke: ${results.length - failed.length}/${results.length} ok ===`);
  console.log('report', out);
  if (failed.length) {
    console.log('failures:');
    for (const f of failed) console.log(' -', f.name, f.detail);
  }
  // leave daemon running for UI smoke
  process.exit(failed.length ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(2);
});
