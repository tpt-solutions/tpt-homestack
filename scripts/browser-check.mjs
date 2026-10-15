// Loads each page of a running tpt-homestack server in headless Chrome or Edge,
// records console errors, exceptions and Content-Security-Policy violations,
// and runs a few real workflows by clicking the page's buttons.
//
// Uses the DevTools protocol directly (no dependencies); needs Node 22+.
//
//   node scripts/browser-check.mjs --base http://127.0.0.1:18090 --password '...'
//   node scripts/browser-check.mjs ... --browser "C:\Program Files\Google\Chrome\Application\chrome.exe"
//
// Exit code 0 when every page is clean and every workflow passes, else 1.

import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, arg, i, all) => {
    if (arg.startsWith('--')) acc.push([arg.slice(2), all[i + 1]]);
    return acc;
  }, []),
);
const base = args.base ?? 'http://127.0.0.1:8080';
const password = args.password;
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

const PAGES = ['/dashboard', '/welcome', '/settings', '/admin/users', '/admin/jobs', '/admin/status', '/admin/plugins', '/admin/remote', '/notes', '/track', '/drop'];
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
  return new Promise((resolve, reject) => {
    ws.onopen = () =>
      resolve({
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
  const profile = mkdtempSync(join(tmpdir(), 'tpt-browser-'));
  const port = 9444;
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

  const report = { browser, base, pages: {}, workflows: {} };
  try {
    await waitFor(async () => (await fetch(`http://127.0.0.1:${port}/json/version`)).ok, 'browser');
    const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    const page = targets.find((t) => t.type === 'page');
    const cdp = await connect(page.webSocketDebuggerUrl);

    // Everything the page reports goes into `logs` until the next visit.
    let logs = [];
    let loadWaiter = null;
    cdp.on((method, params) => {
      if (method === 'Page.loadEventFired' && loadWaiter) {
        const done = loadWaiter;
        loadWaiter = null;
        done();
      } else if (method === 'Log.entryAdded') {
        const e = params.entry;
        const csp = e.source === 'security' || /Content.Security.Policy|Refused to/i.test(e.text);
        if (e.level === 'error' || e.level === 'warning' || csp) {
          logs.push({ level: e.level, csp, text: e.text.slice(0, 300), url: e.url });
        }
      } else if (method === 'Runtime.exceptionThrown') {
        const d = params.exceptionDetails;
        logs.push({ level: 'exception', csp: false, text: (d.exception?.description ?? d.text).slice(0, 300) });
      } else if (method === 'Runtime.consoleAPICalled' && params.type === 'error') {
        logs.push({ level: 'console.error', csp: false, text: params.args.map((a) => a.value ?? a.description).join(' ').slice(0, 300) });
      }
    });
    for (const domain of ['Page', 'Runtime', 'Log', 'Network']) await cdp.send(`${domain}.enable`);

    // Log in through the API, then give the browser the session cookie.
    const login = await fetch(`${base}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'admin', password }),
    });
    if (!login.ok) throw new Error(`login failed: ${login.status}`);
    const setCookie = login.headers.getSetCookie()[0];
    const value = setCookie.split(';')[0].slice('tpt_session='.length);
    await cdp.send('Network.setCookie', { name: 'tpt_session', value, url: base, path: '/' });

    const evaluate = async (expression) => {
      const res = await cdp.send('Runtime.evaluate', {
        expression,
        awaitPromise: true,
        returnByValue: true,
      });
      if (res.exceptionDetails) throw new Error(res.exceptionDetails.exception?.description ?? 'evaluate failed');
      return res.result.value;
    };

    async function visit(path) {
      logs = [];
      const loaded = new Promise((resolve) => {
        loadWaiter = resolve;
      });
      await cdp.send('Page.navigate', { url: `${base}${path}` });
      await Promise.race([loaded, sleep(10000)]);
      // Give the page's own scripts time to run and fetch their data.
      await sleep(800);
      return logs.slice();
    }

    for (const path of PAGES) {
      report.pages[path] = await visit(path);
    }

    // Phone width: a page must not scroll sideways. Measured after the page
    // has loaded and run its scripts, at 390 px wide (a common phone width).
    // Compare against the target width, not window.innerWidth: in mobile
    // emulation Chrome widens the layout viewport to fit wide content, so
    // innerWidth grows with the overflow and would hide it.
    const PHONE_WIDTH = 390;
    await cdp.send('Emulation.setDeviceMetricsOverride', { width: PHONE_WIDTH, height: 844, deviceScaleFactor: 1, mobile: true });
    report.phoneWidth = {};
    for (const path of PAGES) {
      await visit(path);
      const scrollWidth = await evaluate(`document.documentElement.scrollWidth`);
      report.phoneWidth[path] = { scrollWidth, target: PHONE_WIDTH, sideScroll: scrollWidth > PHONE_WIDTH };
    }
    await cdp.send('Emulation.clearDeviceMetricsOverride');

    // Workflow: notes. Open the editor, fill it in, save, and find the note.
    await visit('/notes');
    const clickButton = (text) =>
      evaluate(
        `(() => { const b = [...document.querySelectorAll('button')].find(x => x.textContent.trim() === ${JSON.stringify(text)}); if (!b) return false; b.click(); return true; })()`,
      );
    const clickInCard = (containsText, text) =>
      evaluate(
        `(() => { const card = [...document.querySelectorAll('.card')].find(c => c.innerText.includes(${JSON.stringify(containsText)})); const b = card && [...card.querySelectorAll('button')].find(x => x.textContent.trim() === ${JSON.stringify(text)}); if (!b) return false; b.click(); return true; })()`,
      );
    const title = `browser note ${Date.now()}`;
    const steps = {};
    steps.openEditor = await clickButton('New note');
    await sleep(200);
    await evaluate(`document.getElementById('etitle').value = ${JSON.stringify(title)}; document.getElementById('ebody').value = 'body **bold**';`);
    steps.save = await clickButton('Save');
    await sleep(800);
    steps.listed = await evaluate(`document.getElementById('list').innerText.includes(${JSON.stringify(title)})`);
    report.workflows.notes = { steps, ok: Object.values(steps).every(Boolean), logs: logs.slice() };

    // Workflow: settings. A wrong current password must show the server's
    // refusal on the page. (A successful change would sign the admin out, so
    // that path is not run here.)
    await visit('/settings');
    const pw = {};
    await evaluate(`document.getElementById('cur').value = 'not-the-password'; document.getElementById('new').value = 'another-long-password';`);
    pw.submit = await evaluate(`(() => { const f = document.querySelector('form'); f.requestSubmit(); return true; })()`);
    // Poll rather than sleep: the server checks the current password with
    // Argon2, which takes longer than a fixed delay in a debug build.
    let refusal = '';
    for (let i = 0; i < 60 && !refusal; i++) {
      await sleep(250);
      refusal = await evaluate(`document.getElementById('pwmsg').innerText.trim()`);
    }
    pw.refusalShown = refusal.length > 0;
    pw.stillOnPage = await evaluate(`location.pathname === '/settings'`);
    const pwText = await evaluate(`document.getElementById('pwmsg').innerText`);
    report.workflows.settings = { steps: pw, ok: Object.values(pw).every(Boolean), logs: logs.slice(), pageMessage: pwText };

    // Workflow: track. Create a table with two fields, add a row, delete the table.
    await visit('/track');
    const tname = `browser table ${Date.now()}`;
    const tr = {};
    tr.showForm = await clickButton('New table');
    await sleep(200);
    await evaluate(`
      document.getElementById('tname').value = ${JSON.stringify(tname)};
      const fn = document.querySelectorAll('#fields .fname');
      const fk = document.querySelectorAll('#fields .fkind');
      fn[0].value = 'label'; fk[0].value = 'text';
      fn[1].value = 'amount'; fk[1].value = 'real';`);
    tr.create = await clickButton('Create');
    await sleep(800);
    tr.listed = await evaluate(`document.getElementById('tables').innerText.includes(${JSON.stringify(tname)})`);
    tr.open = await clickInCard(tname, 'Open');
    await sleep(500);
    // Field ids keep counting across tables, so take the first add-row input.
    await evaluate(`(() => { const i = document.querySelector('#rowsview input[id^="add_"]'); if (i) i.value = 'coffee'; return !!i; })()`);
    tr.addRow = await clickButton('Add row');
    await sleep(800);
    tr.rowShown = await evaluate(`document.getElementById('rowsview').innerText.includes('coffee')`);
    await evaluate(`window.confirm = () => true;`);
    tr.deleteTable = await clickInCard(tname, 'Delete');
    await sleep(800);
    tr.gone = await evaluate(`!document.getElementById('tables').innerText.includes(${JSON.stringify(tname)})`);
    // The page reports most failures in #msg rather than the console; keep it.
    const pageMessage = await evaluate(`document.getElementById('msg')?.innerText ?? ''`);
    report.workflows.track = { steps: tr, ok: Object.values(tr).every(Boolean), logs: logs.slice(), pageMessage };

    // Workflow: drop. Upload a real file through the file input, find it in
    // the list, and delete it.
    await visit('/drop');
    const dname = `browser-drop-${Date.now()}.txt`;
    const dpath = join(profile, dname);
    writeFileSync(dpath, 'hello from the browser check\n');
    const dr = {};
    await cdp.send('DOM.enable');
    const doc = await cdp.send('DOM.getDocument', { depth: -1 });
    const input = await cdp.send('DOM.querySelector', { nodeId: doc.root.nodeId, selector: '#file' });
    await cdp.send('DOM.setFileInputFiles', { nodeId: input.nodeId, files: [dpath] });
    dr.upload = await clickButton('Upload');
    await sleep(1200);
    dr.listed = await evaluate(`document.getElementById('list').innerText.includes(${JSON.stringify(dname)})`);
    await evaluate(`window.confirm = () => true;`);
    dr.delete = await clickInCard(dname, 'Delete');
    await sleep(800);
    dr.gone = await evaluate(`!document.getElementById('list').innerText.includes(${JSON.stringify(dname)})`);
    report.workflows.drop = { steps: dr, ok: Object.values(dr).every(Boolean), logs: logs.slice() };

    // Workflow: users. Create a user, change the role, delete the user.
    await visit('/admin/users');
    const uname = `browser-user-${Date.now()}`;
    const us = {};
    await evaluate(`document.getElementById('nu').value = ${JSON.stringify(uname)}; document.getElementById('np').value = 'browser-pass-123';`);
    us.create = await evaluate(`(() => { document.getElementById('nu').form.requestSubmit(); return true; })()`);
    await sleep(800);
    us.listed = await evaluate(`document.querySelector('tbody').innerText.includes(${JSON.stringify(uname)})`);
    us.roleChanged = await evaluate(`(() => {
      const row = [...document.querySelectorAll('tbody tr')].find(r => r.innerText.includes(${JSON.stringify(uname)}));
      const sel = row && row.querySelector('select');
      if (!sel) return false;
      sel.value = 'admin';
      sel.dispatchEvent(new Event('change', { bubbles: true }));
      return true;
    })()`);
    await sleep(800);
    await visit('/admin/users');
    us.roleSaved = await evaluate(`(() => {
      const row = [...document.querySelectorAll('tbody tr')].find(r => r.innerText.includes(${JSON.stringify(uname)}));
      return !!row && row.querySelector('select').value === 'admin';
    })()`);
    // The reset-password modal: open it from the row, set a password, and
    // expect a toast. Replaces the blocking prompt and alert the page used.
    us.resetModalOpened = await evaluate(`(() => {
      const row = [...document.querySelectorAll('tbody tr')].find(r => r.innerText.includes(${JSON.stringify(uname)}));
      const b = row && [...row.querySelectorAll('button')].find(x => x.textContent.trim() === 'Reset pw');
      if (!b) return false;
      b.click();
      return true;
    })()`);
    await sleep(300);
    us.modalOpen = await evaluate(`document.getElementById('pw-modal').open === true`);
    await evaluate(`document.getElementById('pw-new').value = 'reset-by-admin-1';`);
    await evaluate(`[...document.querySelectorAll('#pw-modal button')].find(b => b.textContent.trim() === 'Set password').click(); true`);
    await sleep(800);
    us.toastShown = await evaluate(`[...document.querySelectorAll('#toasts .toast')].some(t => t.textContent.includes('password updated'))`);
    us.deleted = await evaluate(`window.confirm = () => true; (() => {
      const row = [...document.querySelectorAll('tbody tr')].find(r => r.innerText.includes(${JSON.stringify(uname)}));
      const btn = row && [...row.querySelectorAll('button')].find(b => b.textContent.trim() === 'Delete');
      if (!btn) return false;
      btn.click();
      return true;
    })()`);
    await sleep(800);
    await visit('/admin/users');
    us.gone = await evaluate(`![...document.querySelectorAll('tbody tr')].some(r => r.innerText.includes(${JSON.stringify(uname)}))`);
    report.workflows.users = { steps: us, ok: Object.values(us).every(Boolean), logs: logs.slice() };

    // Workflow: first-run wizard. An unfinished wizard sends the admin from
    // the Dashboard to /welcome. Name the server, add an account, and finish.
    // If the wizard is already finished (a re-run), only the page is checked.
    const wz = {};
    await visit('/dashboard');
    const wizardDone = await evaluate(`fetch('/api/settings/server').then(r => r.json()).then(j => j.wizard_done)`);
    wz.redirectedOrFinished = wizardDone ? true : await evaluate(`location.pathname === '/welcome'`);
    if (!wizardDone) {
      const house = `Browser House ${Date.now() % 100000}`;
      await evaluate(`document.getElementById('sn').value = ${JSON.stringify(house)};`);
      await evaluate(`document.getElementById('sn').form.requestSubmit(); true`);
      await sleep(800);
      wz.nameSaved = await evaluate(`document.getElementById('sn-msg').innerText.includes('Saved')`);
      const wuser = `wizard-user-${Date.now() % 100000}`;
      await evaluate(`document.getElementById('nu').value = ${JSON.stringify(wuser)}; document.getElementById('np').value = 'wizard-pass-123';`);
      await evaluate(`document.getElementById('nu').form.requestSubmit(); true`);
      await sleep(800);
      wz.accountListed = await evaluate(`document.getElementById('users').innerText.includes(${JSON.stringify(wuser)})`);
      wz.finished = await clickButton('Finish setup');
      await sleep(1000);
      wz.onDashboard = await evaluate(`location.pathname === '/dashboard'`);
    } else {
      wz.pageShown = await visit('/welcome').then(() => evaluate(`!!document.getElementById('sn')`));
    }
    report.workflows.wizard = { steps: wz, ok: Object.values(wz).every(Boolean), logs: logs.slice() };

    cdp.close();
  } finally {
    proc.kill();
    await sleep(300);
    try {
      rmSync(profile, { recursive: true, force: true });
    } catch {
      // Chrome may still hold files; the temp profile is harmless.
    }
  }

  const pageFailures = Object.entries(report.pages).filter(([, logs]) => logs.length > 0);
  const workflowFailures = Object.entries(report.workflows).filter(([, w]) => !w.ok);
  const sideScrolls = Object.entries(report.phoneWidth).filter(([, m]) => m.sideScroll);
  console.log(JSON.stringify(report, null, 2));
  console.error(
    `pages with problems: ${pageFailures.length}/${PAGES.length}; ` +
      `workflows failing: ${workflowFailures.length}/${Object.keys(report.workflows).length}; ` +
      `pages scrolling sideways at 390 px: ${sideScrolls.length}/${PAGES.length}`,
  );
  process.exit(pageFailures.length || workflowFailures.length || sideScrolls.length ? 1 : 0);
}

main().catch((e) => {
  console.error(e.stack ?? e.message);
  process.exit(2);
});
