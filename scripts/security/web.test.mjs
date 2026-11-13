import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

for (const app of ['desktop', 'android']) {
  test(`${app} webview policy blocks inline scripts and remote code`, () => {
    const config = JSON.parse(readFileSync(`apps/${app}/src-tauri/tauri.conf.json`));
    const csp = config.app.security.csp;
    assert.equal(typeof csp, 'string');
    assert.match(csp, /default-src 'self'/);
    assert.match(csp, /object-src 'none'/);
    assert.match(csp, /base-uri 'none'/);
    const connect = csp.split(';').find(p => p.trim().startsWith('connect-src '));
    assert.equal(connect.trim(), "connect-src 'self' https: ipc: http://ipc.localhost");
    const script = csp.split(';').find(p => p.trim().startsWith('script-src '));
    assert.equal(script.trim(), "script-src 'self'");
    const source = readFileSync(`apps/${app}/src-tauri/src/lib.rs`, 'utf8');
    assert.match(source, /asset\.csp_header\(\)/);
  });
}

test('desktop retains signature verification', () => {
  const config = JSON.parse(readFileSync('apps/desktop/src-tauri/tauri.conf.json'));
  assert.equal(config.plugins.updater.requireSignedVersion, true);
  assert.ok(config.plugins.updater.pubkey);
  assert.equal(config.plugins.updater.endpoints, undefined);
});

test('desktop setup uses no inline script', () => {
  const html = readFileSync('apps/desktop/src-tauri/src/setup.html', 'utf8');
  assert.match(html, /<script src="\/_desktop\/setup\.js"><\/script>/);
  assert.doesNotMatch(html, /<script>/);
});
