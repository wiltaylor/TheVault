'use strict';
// Run inside the disposable browser image. Only fabricated credentials.
const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const crypto = require('node:crypto');
const fs = require('node:fs/promises');

test('secret fill uses signed profile, exact origin and memory-only transport', { timeout: 120000 }, async () => {
  const token = process.env.BROWSER_TOKEN;
  const profile = 'secret-fill-image';
  const signature = crypto.createHmac('sha256', token).update('thevault-browser-profile-v1:' + profile).digest('hex');
  const headers = { authorization: `Bearer ${token}`, 'content-type': 'application/json',
    'x-vault-agent': profile, 'x-vault-agent-signature': signature };
  const base = 'http://127.0.0.1:7700';
  const received = [];
  const site = http.createServer(async (req, res) => {
    if (req.method === 'POST') {
      let text = '';
      for await (const chunk of req) text += chunk;
      received.push(text);
      res.end('ok');
    } else {
      res.setHeader('content-type', 'text/html');
      res.end('<input id="password" type="password"><script>document.querySelector("input").addEventListener("input", e => fetch("/filled", {method:"POST", body:e.target.value}))</script>');
    }
  });
  await new Promise(resolve => site.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${site.address().port}`;
  const post = async (route, body, override = headers) => {
    const response = await fetch(base + route, { method: 'POST', headers: override, body: JSON.stringify(body) });
    return { status: response.status, body: await response.json() };
  };
  const run = async (command, enabled) => {
    const response = await post('/run', { command, timeout_s: 90, secret_browser: enabled });
    assert.equal(response.status, 200);
    assert.equal(response.body.exit, 0, 'browser setup failed');
    return response.body;
  };
  let monitoring = true;
  const observed = [];
  const monitor = async () => {
    while (monitoring) {
      for (const pid of await fs.readdir('/proc')) {
        if (!/^\d+$/.test(pid) || Number(pid) === process.pid) continue;
        for (const file of ['cmdline', 'environ']) {
          try { observed.push(await fs.readFile(`/proc/${pid}/${file}`, 'utf8')); } catch {}
        }
      }
      await new Promise(resolve => setTimeout(resolve, 5));
    }
  };
  let watching;
  try {
    await run('true', false);
    const home = `/home/agent/profiles/${profile}/home`;
    assert.equal(await fs.readFile(home + '/.secret-browser/.playwright/cli.config.json', 'utf8'), '{}');
    await run(`playwright-cli open ${origin} --headed --persistent`, true);
    const request = { session: 'default', target: '#password' };
    const page = await post('/secret/origin', request);
    assert.equal(page.status, 200);
    assert.equal(page.body.origin, origin);
    // Construct the fixture in this process, never put it in a command line.
    const value = ['fabricated', crypto.randomBytes(16).toString('hex')].join('-');
    const fill = { ...request, origin, value };
    assert.equal((await post('/secret/fill', fill, {})).status, 401);
    const mismatch = await post('/secret/fill', { ...fill, origin: 'https://other.example' });
    assert.deepEqual(mismatch.body, { status: 'origin_mismatch' });
    assert.equal(received.length, 0);
    watching = monitor();
    for (let i = 0; i < 3; i++) {
      const result = await post('/secret/fill', fill);
      assert.equal(result.status, 200);
      assert.deepEqual(result.body, { status: 'filled' });
      assert.equal((await post('/secret/origin', request)).body.origin, origin);
    }
    for (let i = 0; i < 50 && received.length < 3; i++) await new Promise(resolve => setTimeout(resolve, 10));
    assert.equal(received.length, 3);
    assert(received.every(text => text === value), 'page did not receive the fabricated value');
    monitoring = false;
    await watching;
    assert(observed.some(text => text.includes('thevault-secret-fill')), 'bridge process was not sampled');
    assert(observed.every(text => !text.includes(value)), 'credential leaked to argv or environment');
    // No CLI command is run after a fill before this check. Include snapshots,
    // tracing, session and config files, not just the service response.
    const scan = async dir => {
      for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
        const name = `${dir}/${entry.name}`;
        if (entry.isDirectory()) await scan(name);
        else if (entry.isFile()) {
          const bytes = await fs.readFile(name).catch(() => Buffer.alloc(0));
          assert(!bytes.includes(Buffer.from(value)), 'credential was written to a profile file');
        }
      }
    };
    await scan(`/home/agent/profiles/${profile}`);
    // Losing the grant disables configuration and terminates old listeners.
    await run('true', false);
    assert.equal((await post('/secret/origin', request)).status, 400);
  } finally {
    monitoring = false;
    if (watching) await watching;
    await fetch(base + '/profile', { method: 'DELETE', headers });
    await new Promise(resolve => site.close(resolve));
  }
});
