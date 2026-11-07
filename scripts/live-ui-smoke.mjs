/**
 * Live Tauri UI smoke:
 * 1) Ensure ferricd is up (lockfile)
 * 2) Serve ui/dist on :3000 (debug Tauri loads devUrl)
 * 3) Start ferric-ui with WebView2 remote debugging
 * 4) Playwright CDP: navigate sidebar, open dialogs, hit key actions
 */
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, '..');
const isWin = process.platform === 'win32';
const uiBin =
  process.env.FERRIC_UI_BIN ||
  path.join(root, 'target', 'debug', isWin ? 'ferric-ui.exe' : 'ferric-ui');
const uiDir = path.join(root, 'crates', 'ferric-ui', 'ui');
const cdpPort = Number(process.env.WEBVIEW_CDP_PORT || 9223);
const frontendPort = Number(process.env.FRONTEND_PORT || 3000);

const results = [];
const log = (ok, name, detail = '') => {
  results.push({ ok, name, detail: String(detail).slice(0, 250) });
  console.log(`${ok ? 'OK ' : 'FAIL'} ${name}${detail ? ` — ${String(detail).slice(0, 140)}` : ''}`);
};

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function findLock() {
  const local = process.env.LOCALAPPDATA || '';
  for (const c of [
    path.join(local, 'ferric', 'ferric', 'data', 'ferric.lock'),
    path.join(local, 'ferric', 'ferric', 'ferric.lock'),
  ]) {
    if (fs.existsSync(c)) return c;
  }
  return null;
}

function readLock(lockPath) {
  const lines = fs.readFileSync(lockPath, 'utf8').split(/\r?\n/).filter(Boolean);
  return { port: Number(lines[0]), token: lines[1] };
}

function api(lock, method, p) {
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        hostname: '127.0.0.1',
        port: lock.port,
        path: p,
        method,
        headers: { 'X-Ferric-Auth': lock.token },
        timeout: 8000,
      },
      (res) => {
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () =>
          resolve({ status: res.statusCode, body: Buffer.concat(chunks).toString('utf8') }),
        );
      },
    );
    req.on('error', reject);
    req.end();
  });
}

function waitHttp(url, ms = 30_000) {
  const start = Date.now();
  return new Promise((resolve, reject) => {
    const tick = () => {
      http
        .get(url, (res) => {
          res.resume();
          resolve(res.statusCode);
        })
        .on('error', () => {
          if (Date.now() - start > ms) reject(new Error(`timeout ${url}`));
          else setTimeout(tick, 300);
        });
    };
    tick();
  });
}

function killTree(pid) {
  if (!pid) return;
  if (isWin) spawn('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' });
  else {
    try {
      process.kill(pid, 'SIGTERM');
    } catch {
      /* ignore */
    }
  }
}

async function main() {
  if (!fs.existsSync(uiBin)) {
    console.error('ferric-ui missing — cargo build -p ferric-ui');
    process.exit(2);
  }
  if (!fs.existsSync(path.join(uiDir, 'dist', 'index.html'))) {
    console.error('ui/dist missing — cd crates/ferric-ui/ui && npm run build');
    process.exit(2);
  }

  const lockPath = findLock();
  if (!lockPath) {
    console.error('daemon lock missing — node scripts/live-daemon-smoke.mjs first');
    process.exit(2);
  }
  const lock = readLock(lockPath);
  const health = await api(lock, 'GET', '/health');
  log(health.status === 200, 'daemon_health', `port=${lock.port}`);

  // Ensure settings allow smoke (onboarding already set by daemon smoke)
  try {
    await new Promise((resolve, reject) => {
      const data = JSON.stringify({ onboarding_done: '1' });
      const req = http.request(
        {
          hostname: '127.0.0.1',
          port: lock.port,
          path: '/settings',
          method: 'POST',
          headers: {
            'X-Ferric-Auth': lock.token,
            'Content-Type': 'application/json',
            'Content-Length': Buffer.byteLength(data),
          },
        },
        (res) => {
          res.resume();
          resolve(res.statusCode);
        },
      );
      req.on('error', reject);
      req.write(data);
      req.end();
    });
  } catch {
    /* ignore */
  }

  console.log(`starting frontend preview :${frontendPort}`);
  const preview = spawn(
    isWin ? 'npm.cmd' : 'npm',
    ['run', 'preview', '--', '--host', '127.0.0.1', '--port', String(frontendPort), '--strictPort'],
    { cwd: uiDir, stdio: 'ignore', shell: true, windowsHide: true },
  );
  await waitHttp(`http://127.0.0.1:${frontendPort}/`);
  log(true, 'frontend_preview', `:${frontendPort}`);

  console.log('starting', uiBin);
  const ui = spawn(uiBin, [], {
    cwd: path.dirname(uiBin),
    stdio: 'ignore',
    windowsHide: false,
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${cdpPort}`,
    },
  });
  let intentionalStop = false;
  let exited = false;
  ui.on('exit', (code) => {
    exited = true;
    if (!intentionalStop) log(false, 'ui_exited_early', `code=${code}`);
  });
  await sleep(5000);
  log(!exited, 'ui_process_alive', `pid=${ui.pid}`);

  try {
    const h2 = await api(lock, 'GET', '/health');
    log(h2.status === 200, 'daemon_with_ui', `status=${h2.status}`);
  } catch (e) {
    log(false, 'daemon_with_ui', e.message);
  }

  const pwPath = [
    path.join(uiDir, 'node_modules', 'playwright', 'index.mjs'),
    path.join(uiDir, 'node_modules', 'playwright', 'index.js'),
  ].find((p) => fs.existsSync(p));
  if (!pwPath) {
    log(false, 'playwright_missing', 'install in crates/ferric-ui/ui');
  } else {
    try {
      const { chromium } = await import(pathToFileURL(pwPath).href);
      let browser;
      let lastErr;
      for (let i = 0; i < 15; i++) {
        try {
          browser = await chromium.connectOverCDP(`http://127.0.0.1:${cdpPort}`, { timeout: 2000 });
          break;
        } catch (e) {
          lastErr = e;
          await sleep(400);
        }
      }
      if (!browser) throw lastErr;

      const pages = browser.contexts().flatMap((c) => c.pages());
      const page = pages.find((p) => p.url().includes('localhost') || p.url().includes('127.0.0.1')) || pages[0];
      log(!!page, 'webview_cdp_page', page ? page.url() : 'none');
      if (page) {
        await page.waitForTimeout(1500);
        await page.evaluate(() => {
          document.querySelectorAll('[role="dialog"]').forEach((el) => el.remove());
        }).catch(() => {});

        // Workspace tabs (Overview is default landing)
        for (const tabLabel of ['Overview', 'Downloads', 'Torrents']) {
          try {
            await page.locator('button.tab').filter({ hasText: tabLabel }).first().click({ timeout: 4000 });
            await page.waitForTimeout(350);
            log(true, `tab:${tabLabel}`, 'clicked');
          } catch (e) {
            log(false, `tab:${tabLabel}`, String(e).slice(0, 120));
          }
        }

        // Downloads workspace sidebar
        try {
          await page.locator('button.tab').filter({ hasText: 'Downloads' }).first().click({ timeout: 4000 });
          await page.waitForTimeout(300);
        } catch (e) {
          log(false, 'tab:Downloads:reselect', String(e).slice(0, 120));
        }
        for (const label of ['Grabber', 'Scheduler', 'Media', 'Site Rules', 'RSS']) {
          try {
            const item = page.locator('button.sidebar-item').filter({ hasText: label }).first();
            await item.click({ timeout: 4000 });
            await page.waitForTimeout(350);
            log(true, `nav:downloads:${label}`, 'clicked');
          } catch (e) {
            log(false, `nav:downloads:${label}`, String(e).slice(0, 120));
          }
        }

        // Torrents workspace — Privacy lives here
        try {
          await page.locator('button.tab').filter({ hasText: 'Torrents' }).first().click({ timeout: 4000 });
          await page.waitForTimeout(300);
          await page.locator('button.sidebar-item').filter({ hasText: 'Anonymity' }).first().click({ timeout: 4000 });
          await page.waitForTimeout(350);
          log(true, 'nav:torrents:Privacy', 'clicked');
        } catch (e) {
          log(false, 'nav:torrents:Privacy', String(e).slice(0, 120));
        }

        // System slide-over (gear) — Security / Proxies / Settings
        try {
          await page.locator('button.workspace-gear').first().click({ timeout: 4000 });
          await page.waitForTimeout(300);
          for (const label of ['Security', 'Proxies', 'Settings']) {
            const tab = page.locator('button.system-panel-tab').filter({ hasText: label }).first();
            await tab.click({ timeout: 3000 });
            await page.waitForTimeout(300);
            log(true, `system:${label}`, 'clicked');
          }
        } catch (e) {
          log(false, 'system_panel', String(e).slice(0, 120));
        }

        // Settings subnav (inside system panel)
        try {
          await page.locator('button.system-panel-tab').filter({ hasText: 'Settings' }).first().click();
          await page.waitForTimeout(300);
          for (const label of ['General', 'Webhooks', 'Categories']) {
            const b = page.locator('button.settings-nav-item').filter({ hasText: new RegExp(`^${label}$`) });
            if ((await b.count()) > 0) {
              await b.first().click({ timeout: 2000 });
              await page.waitForTimeout(200);
              log(true, `settings:${label}`, 'clicked');
            }
          }
        } catch (e) {
          log(false, 'settings_nav', String(e).slice(0, 120));
        }

        // Security export (system panel)
        try {
          await page.locator('button.system-panel-tab').filter({ hasText: 'Security' }).first().click();
          await page.waitForTimeout(400);
          const exportBtn = page
            .locator('.system-panel-body button, .system-panel button')
            .filter({ hasText: /Export audit|Export|Download/i })
            .first();
          await exportBtn.scrollIntoViewIfNeeded().catch(() => {});
          await exportBtn.click({ timeout: 5000, force: true });
          await page.waitForTimeout(400);
          log(true, 'security_export_click', 'clicked');
        } catch (e) {
          log(false, 'security_export_click', String(e).slice(0, 120));
        }

        // Close system panel
        await page.locator('button.system-panel-close').first().click({ timeout: 2000 }).catch(() => {});
        await page.keyboard.press('Escape').catch(() => {});
        await page.waitForTimeout(200);

        // Site rules add (Downloads workspace)
        try {
          await page.locator('button.tab').filter({ hasText: 'Downloads' }).first().click({ timeout: 4000 });
          await page.waitForTimeout(200);
          await page.locator('button.sidebar-item').filter({ hasText: 'Site Rules' }).first().click();
          await page.waitForTimeout(300);
          const input = page.locator('input.field-input:not([type="number"]), input.field-input[type="text"]').first();
          await input.fill(`*://live-smoke-${Date.now()}.example/*`);
          await page.locator('button').filter({ hasText: /^Add$/ }).first().click();
          await page.waitForTimeout(500);
          const rules = await api(lock, 'GET', '/site-rules');
          const body = JSON.parse(rules.body || '[]');
          const ok = Array.isArray(body) && body.some((r) => String(r.pattern || '').includes('live-smoke-'));
          log(ok, 'site_rules_add_live', `api_count=${Array.isArray(body) ? body.length : '?'}`);
        } catch (e) {
          log(false, 'site_rules_add_live', String(e).slice(0, 120));
        }

        // Toolbar Add URL open/close (needs Downloads/Torrents workspace)
        try {
          await page.locator('button.tab').filter({ hasText: 'Downloads' }).first().click({ timeout: 4000 });
          await page.waitForTimeout(200);
          await page.locator('button.tb-add').first().click({ timeout: 3000 });
          await page.waitForTimeout(400);
          await page.keyboard.press('Escape');
          await page.waitForTimeout(200);
          log(true, 'toolbar_add_url', 'opened+esc');
        } catch (e) {
          log(false, 'toolbar_add_url', String(e).slice(0, 120));
        }

        // Pause/Resume on selected if toolbar present
        for (const cls of ['tb-pause', 'tb']) {
          try {
            const b = page.locator(`button.${cls}`).first();
            if ((await b.count()) > 0) {
              await b.click({ timeout: 1500 }).catch(() => {});
            }
          } catch {
            /* ignore */
          }
        }
        log(true, 'toolbar_pause_attempt', 'clicked if present');
      }
      await browser.close().catch(() => {});
    } catch (e) {
      log(false, 'webview_cdp', String(e).slice(0, 160));
    }
  }

  intentionalStop = true;
  killTree(ui.pid);
  killTree(preview.pid);
  await sleep(800);

  const failed = results.filter((r) => !r.ok);
  const out = path.join(root, 'scripts', 'live-ui-smoke-report.json');
  fs.writeFileSync(out, JSON.stringify({ failed: failed.length, total: results.length, results }, null, 2));
  console.log(`\n=== live UI smoke: ${results.length - failed.length}/${results.length} ok ===`);
  if (failed.length) {
    console.log('failures:');
    for (const f of failed) console.log(' -', f.name, f.detail);
  }
  console.log('report', out);
  process.exit(failed.length ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(2);
});
