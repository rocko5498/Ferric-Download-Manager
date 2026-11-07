/**
 * FQT-115: E2E smoke test — ferric-qt + ferricd HTTP fixture.
 * Starts ferricd, hits key API endpoints, verifies responses.
 * Usage: node scripts/e2e-ferric-smoke.mjs
 */
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, '..');
const isWin = process.platform === 'win32';
const ferricdBin = process.env.FERRICD_BIN || path.join(root, 'target', 'debug', isWin ? 'ferricd.exe' : 'ferricd');

let passed = 0;
let failed = 0;

function assert(cond, msg) {
  if (cond) { passed++; console.log(`  PASS: ${msg}`); }
  else { failed++; console.error(`  FAIL: ${msg}`); }
}

function findLockPath() {
  const local = process.env.LOCALAPPDATA || process.env.HOME || '';
  const candidates = [
    path.join(local, 'ferric', 'ferric', 'data', 'ferric.lock'),
    path.join(local, 'ferric', 'ferric', 'ferric.lock'),
  ];
  for (const c of candidates) { if (fs.existsSync(c)) return c; }
  return candidates[0];
}

function readLock() {
  const lockPath = findLockPath();
  if (!fs.existsSync(lockPath)) return null;
  const content = fs.readFileSync(lockPath, 'utf-8');
  const portMatch = content.match(/port:\s*(\d+)/);
  const tokenMatch = content.match(/token:\s*(\S+)/);
  if (!portMatch || !tokenMatch) return null;
  return { port: parseInt(portMatch[1]), token: tokenMatch[1] };
}

function apiGet(port, token, path) {
  return new Promise((resolve, reject) => {
    const req = http.get(`http://127.0.0.1:${port}${path}`, { headers: { 'X-Ferric-Auth': token } }, (res) => {
      let data = '';
      res.on('data', (chunk) => data += chunk);
      res.on('end', () => resolve({ status: res.statusCode, body: data }));
    });
    req.on('error', reject);
    req.setTimeout(5000, () => { req.destroy(); reject(new Error('timeout')); });
  });
}

function apiPost(port, token, path, body) {
  return new Promise((resolve, reject) => {
    const payload = JSON.stringify(body);
    const req = http.request(`http://127.0.0.1:${port}${path}`, {
      method: 'POST', headers: { 'X-Ferric-Auth': token, 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) }
    }, (res) => {
      let data = '';
      res.on('data', (chunk) => data += chunk);
      res.on('end', () => resolve({ status: res.statusCode, body: data }));
    });
    req.on('error', reject);
    req.write(payload);
    req.end();
  });
}

async function runTests() {
  console.log('FQT-115: E2E smoke test\n');

  // Check ferricd binary exists
  assert(fs.existsSync(ferricdBin), `ferricd binary exists at ${ferricdBin}`);

  // Read lock file
  const lock = readLock();
  if (!lock) {
    console.error('  FAIL: No lock file found — daemon not running. Start ferricd before running smoke tests.');
    console.log(`\nResults: ${passed} passed, ${failed + 1} failed`);
    process.exit(1);
  }
  const { port, token } = lock;
  console.log(`  Daemon at port ${port}\n`);

  // Test 1: Health
  try {
    const r = await apiGet(port, token, '/health');
    assert(r.status === 200, `GET /health → 200 (got ${r.status})`);
    const body = JSON.parse(r.body);
    assert(body.version !== undefined, `health has version field`);
  } catch (e) { assert(false, `GET /health failed: ${e.message}`); }

  // Test 2: Settings
  try {
    const r = await apiGet(port, token, '/settings');
    assert(r.status === 200, `GET /settings → 200`);
  } catch (e) { assert(false, `GET /settings failed: ${e.message}`); }

  // Test 3: Stats
  try {
    const r = await apiGet(port, token, '/stats');
    assert(r.status === 200, `GET /stats → 200`);
    const body = JSON.parse(r.body);
    assert('active_downloads' in body, `stats has active_downloads`);
  } catch (e) { assert(false, `GET /stats failed: ${e.message}`); }

  // Test 4: Downloads list
  try {
    const r = await apiGet(port, token, '/downloads');
    assert(r.status === 200, `GET /downloads → 200`);
    assert(Array.isArray(JSON.parse(r.body)), `downloads is array`);
  } catch (e) { assert(false, `GET /downloads failed: ${e.message}`); }

  // Test 5: Torrents list
  try {
    const r = await apiGet(port, token, '/torrents');
    assert(r.status === 200, `GET /torrents → 200`);
    assert(Array.isArray(JSON.parse(r.body)), `torrents is array`);
  } catch (e) { assert(false, `GET /torrents failed: ${e.message}`); }

  // Test 6: Proxies list
  try {
    const r = await apiGet(port, token, '/proxies');
    assert(r.status === 200, `GET /proxies → 200`);
  } catch (e) { assert(false, `GET /proxies failed: ${e.message}`); }

  // Test 7: RSS feeds
  try {
    const r = await apiGet(port, token, '/rss/feeds');
    assert(r.status === 200, `GET /rss/feeds → 200`);
  } catch (e) { assert(false, `GET /rss/feeds failed: ${e.message}`); }

  // Test 8: Webhooks
  try {
    const r = await apiGet(port, token, '/webhooks');
    assert(r.status === 200, `GET /webhooks → 200`);
  } catch (e) { assert(false, `GET /webhooks failed: ${e.message}`); }

  // Test 9: Categories
  try {
    const r = await apiGet(port, token, '/categories');
    assert(r.status === 200, `GET /categories → 200`);
  } catch (e) { assert(false, `GET /categories failed: ${e.message}`); }

  // Test 10: Privacy dashboard
  try {
    const r = await apiGet(port, token, '/privacy/dashboard');
    assert(r.status === 200, `GET /privacy/dashboard → 200`);
  } catch (e) { assert(false, `GET /privacy/dashboard failed: ${e.message}`); }

  // Test 11: Plugins
  try {
    const r = await apiGet(port, token, '/plugins');
    assert(r.status === 200, `GET /plugins → 200`);
  } catch (e) { assert(false, `GET /plugins failed: ${e.message}`); }

  console.log(`\nResults: ${passed} passed, ${failed} failed`);
  process.exit(failed > 0 ? 1 : 0);
}

runTests().catch(e => { console.error(e); process.exit(1); });
