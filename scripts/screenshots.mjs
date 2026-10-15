// Takes screenshots of the main pages of a running tpt-homestack server, for the
// non-developer guides. Uses the same headless Chrome or Edge and DevTools
// protocol as browser-check.mjs.
//
//   node scripts/screenshots.mjs --base http://127.0.0.1:18090 --password '...' --out docs/screenshots
//
// Run it against a scratch server with sample data, not a household server:
// the images show whatever the pages show.
//
// Exit code 0 when every picture was written, else 1.

import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, arg, i, all) => {
    if (arg.startsWith('--')) acc.push([arg.slice(2), all[i + 1]]);
    return acc;
  }, []),
);
const base = args.base ?? 'http://127.0.0.1:8080';
const password = args.password;
const outDir = resolve(args.out ?? 'docs/screenshots');
if (!password) {
  console.error('--password is required');
  process.exit(2);
}

const candidates = [
  args.browser,
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
  '/usr/bin/chromium',
  '/usr/bin/google-chrome',
].filter(Boolean);
const browser = candidates.find((p) => existsSync(p));
if (!browser) {
  console.error('no Chrome or Edge found; pass --browser');
  process.exit(2);
}

// Each picture: a file name, the page, and the viewport it is taken at.
const SHOTS = [
  { file: 'login.png', path: '/login', width: 1100, height: 700, signedIn: false },
  { file: 'welcome.png', path: '/welcome', width: 1100, height: 900 },
  { file: 'dashboard.png', path: '/dashboard', width: 1100, height: 700 },
  { file: 'settings-phone-card.png', path: '/settings', width: 1100, height: 1000 },
  { file: 'admin-users.png', path: '/admin/users', width: 1100, height: 700 },
  { file: 'admin-plugins.png', path: '/admin/plugins', width: 1100, height: 800 },
  { file: 'admin-status.png', path: '/admin/status', width: 1100, height: 1000 },
  { file: 'admin-remote.png', path: '/admin/remote', width: 1100, height: 700 },
  { file: 'phone-dashboard.png', path: '/dashboard', width: 390, height: 844 },
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function waitFor(fn, what, timeoutMs = 15000) {
  const end = Date.now() + timeoutMs;
  for (;;) {
    try {
      const value = await fn();
      if (value) return value;
    } catch {
      // not up yet
    }
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}

/** A tiny DevTools protocol client over the built-in WebSocket. */
function connect(url) {
  const ws = new WebSocket(url);
  let nextId = 1;
  const pending = new Map();
  const listeners = [];
  ws.onmessage = (event) => {
    const msg = JSON.parse(event.data);
    if (msg.id !== undefined) {
      const done = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) done.reject(new Error(msg.error.message));
      else done.resolve(msg.result);
    } else {
      for (const fn of listeners) fn(msg.method, msg.params);
    }
  };
  return new Promise((resolvePromise, reject) => {
    ws.onopen = () =>
      resolvePromise({
        send(method, params = {}) {
          const id = nextId++;
          ws.send(JSON.stringify({ id, method, params }));
          return new Promise((res, rej) => pending.set(id, { resolve: res, reject: rej }));
        },
        on(fn) {
          listeners.push(fn);
        },
        close() {
          ws.close();
        },
      });
    ws.onerror = () => reject(new Error(`cannot connect to ${url}`));
  });
}

async function main() {
  mkdirSync(outDir, { recursive: true });
  const profile = mkdtempSync(join(tmpdir(), 'tpt-shots-'));
  const port = 9445;
  const proc = spawn(
    browser,
    [
      '--headless=new',
      `--remote-debugging-port=${port}`,
      `--user-data-dir=${profile}`,
      '--no-first-run',
      '--disable-gpu',
      '--no-sandbox',
      'about:blank',
    ],
    { stdio: 'ignore' },
  );

  const written = [];
  try {
    await waitFor(async () => (await fetch(`http://127.0.0.1:${port}/json/version`)).ok, 'browser');
    const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    const page = targets.find((t) => t.type === 'page');
    const cdp = await connect(page.webSocketDebuggerUrl);

    let loadWaiter = null;
    cdp.on((method) => {
      if (method === 'Page.loadEventFired' && loadWaiter) {
        const done = loadWaiter;
        loadWaiter = null;
        done();
      }
    });
    await cdp.send('Page.enable');

    const login = await fetch(`${base}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'admin', password }),
    });
    if (!login.ok) throw new Error(`login failed: ${login.status}`);
    const value = login.headers.getSetCookie()[0].split(';')[0].slice('tpt_session='.length);

    let signedIn = false;
    for (const shot of SHOTS) {
      if (shot.signedIn !== false && !signedIn) {
        await cdp.send('Network.enable');
        await cdp.send('Network.setCookie', { name: 'tpt_session', value, url: base, path: '/' });
        signedIn = true;
      }
      if (shot.signedIn === false && signedIn) {
        await cdp.send('Network.clearBrowserCookies');
        signedIn = false;
      }
      await cdp.send('Emulation.setDeviceMetricsOverride', {
        width: shot.width,
        height: shot.height,
        deviceScaleFactor: 1,
        mobile: shot.width < 500,
      });
      const loaded = new Promise((resolvePromise) => {
        loadWaiter = resolvePromise;
      });
      await cdp.send('Page.navigate', { url: `${base}${shot.path}` });
      await Promise.race([loaded, sleep(10000)]);
      await sleep(900);
      const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' });
      const file = join(outDir, shot.file);
      writeFileSync(file, Buffer.from(data, 'base64'));
      written.push(file);
    }
    cdp.close();
  } finally {
    // Wait for the browser to let go of its profile before removing it.
    const exited = new Promise((resolvePromise) => proc.once('exit', resolvePromise));
    proc.kill();
    await Promise.race([exited, sleep(3000)]);
    try {
      rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 300 });
    } catch {
      console.warn(`could not remove the browser profile ${profile}; delete it by hand`);
    }
  }
  for (const file of written) console.log(`wrote ${file}`);
  console.log(`${written.length}/${SHOTS.length} screenshots written`);
  return written.length === SHOTS.length ? 0 : 1;
}

main()
  .then((code) => process.exit(code))
  .catch((err) => {
    console.error(err.message);
    process.exit(1);
  });
