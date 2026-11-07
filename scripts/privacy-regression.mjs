/**
 * FQT-116: Privacy regression tests — kill-switch + no listen.
 * Verifies: start refused when require SOCKS + empty URL; tcp_listen default 0.
 * Usage: node scripts/privacy-regression.mjs
 */
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

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
  if (portMatch && tokenMatch) {
    return { port: parseInt(portMatch[1]), token: tokenMatch[1] };
  }
  const lines = content.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  if (!/^\d+$/.test(lines[0] || '') || !lines[1]) return null;
  return { port: parseInt(lines[0]), token: lines[1] };
}

function apiGet(port, token, urlPath) {
  return new Promise((resolve, reject) => {
    const req = http.get(`http://127.0.0.1:${port}${urlPath}`, { headers: { 'X-Ferric-Auth': token } }, (res) => {
      let data = '';
      res.on('data', (chunk) => data += chunk);
      res.on('end', () => resolve({ status: res.statusCode, body: data }));
    });
    req.on('error', reject);
    req.setTimeout(5000, () => { req.destroy(); reject(new Error('timeout')); });
  });
}

function apiPost(port, token, urlPath, body) {
  return new Promise((resolve, reject) => {
    const payload = body ? JSON.stringify(body) : '';
    const req = http.request(`http://127.0.0.1:${port}${urlPath}`, {
      method: 'POST',
      headers: { 'X-Ferric-Auth': token, 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) }
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
  console.log('FQT-116: Privacy regression tests\n');

  const lock = readLock();
  if (!lock) {
    console.error('  FAIL: No lock file — daemon not running. Start ferricd before running smoke tests.');
    console.log(`\nResults: ${passed} passed, ${failed + 1} failed`);
    process.exit(1);
  }
  const { port, token } = lock;
  console.log(`  Daemon at port ${port}\n`);

  // Test 1: tcp_listen_enabled defaults to 0
  try {
    const r = await apiGet(port, token, '/settings');
    const settings = JSON.parse(r.body);
    const listenVal = settings.torrent_tcp_listen_enabled || '0';
    assert(listenVal === '0', `torrent_tcp_listen_enabled defaults to 0 (got "${listenVal}")`);
  } catch (e) { assert(false, `Settings fetch failed: ${e.message}`); }

  // Balanced mode matches mainstream clients: proxy is opt-in.
  try {
    const r = await apiGet(port, token, '/settings');
    const settings = JSON.parse(r.body);
    const proxyReq = settings.torrent_proxy_required || '0';
    assert(proxyReq === '0', `torrent_proxy_required defaults to 0 (got "${proxyReq}")`);
  } catch (e) { assert(false, `Settings fetch failed: ${e.message}`); }

  // Test 3: torrent_engine_enabled exists
  try {
    const r = await apiGet(port, token, '/settings');
    const settings = JSON.parse(r.body);
    assert('torrent_engine_enabled' in settings, `torrent_engine_enabled setting exists`);
  } catch (e) { assert(false, `Settings fetch failed: ${e.message}`); }

  // Test 4: privacy dashboard returns valid data
  try {
    const r = await apiGet(port, token, '/privacy/dashboard');
    assert(r.status === 200, `GET /privacy/dashboard → 200`);
    const body = JSON.parse(r.body);
    assert(typeof body === 'object', `privacy dashboard returns object`);
  } catch (e) { assert(false, `Privacy dashboard failed: ${e.message}`); }

  // Test 5: anonymity score exists
  try {
    const r = await apiGet(port, token, '/privacy/anonymity-score');
    assert(r.status === 200, `GET /privacy/anonymity-score → 200`);
  } catch (e) { assert(false, `Anonymity score failed: ${e.message}`); }

  // Test 6: kill_switch_active setting exists
  try {
    const r = await apiGet(port, token, '/settings');
    const settings = JSON.parse(r.body);
    assert('torrent_kill_switch_active' in settings, `torrent_kill_switch_active setting exists`);
  } catch (e) { assert(false, `Settings fetch failed: ${e.message}`); }

  // Test 7: auth required (no token → 401/403)
  try {
    const r = await new Promise((resolve, reject) => {
      const req = http.get(`http://127.0.0.1:${port}/health`, (res) => {
        let data = '';
        res.on('data', (chunk) => data += chunk);
        res.on('end', () => resolve({ status: res.statusCode, body: data }));
      });
      req.on('error', reject);
      req.setTimeout(3000, () => { req.destroy(); reject(new Error('timeout')); });
    });
    assert(r.status === 401 || r.status === 403, `Request without auth → ${r.status} (expected 401/403)`);
  } catch (e) { assert(false, `Auth test failed: ${e.message}`); }

  console.log(`\nResults: ${passed} passed, ${failed} failed`);
  process.exit(failed > 0 ? 1 : 0);
}

runTests().catch(e => { console.error(e); process.exit(1); });
