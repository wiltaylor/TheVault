'use strict';
// Pinned @playwright/cli 0.1.22 / playwright-core 1.64.0-alpha-1790635538000.
// stdin is the only credential input. No console logging, CLI fill, or Playwright
// evaluation receives a value. Raw CDP bypasses CLI code generation and tracing.
const fs = require('node:fs/promises');
const path = require('node:path');
const core = '/usr/local/lib/node_modules/@playwright/cli/node_modules/playwright-core/lib';

async function pageSocket(sessionName) {
  const { Registry, createClientInfo } = require(`${core}/tools/cli-client/registry.js`);
  const info = createClientInfo();
  const registry = await Registry.load();
  const entry = registry.entry(info, sessionName);
  if (!entry || entry.config.version !== '1.64.0-alpha-1790635538000') throw new Error();
  const root = await fs.realpath(process.env.HOME);
  const profile = await fs.realpath(entry.config.browser.userDataDir);
  if (!profile.startsWith(root + path.sep)) throw new Error();
  const active = await fs.readFile(path.join(profile, 'DevToolsActivePort'), 'utf8');
  const port = Number(active.split('\n')[0]);
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error();
  // Do not even query the CLI after a fill: automatic snapshots could capture
  // the old value. A single page makes the target unambiguous without the CLI.
  const response = await fetch(`http://127.0.0.1:${port}/json/list`, { redirect: 'error' });
  const pages = (await response.json()).filter(target => target.type === 'page');
  if (pages.length !== 1 || !/^[A-Fa-f0-9]{32}$/.test(pages[0].id)) throw new Error();
  return `ws://127.0.0.1:${port}/devtools/page/${pages[0].id}`;
}

async function connect(url) {
  const socket = new WebSocket(url);
  await new Promise((resolve, reject) => {
    socket.onopen = resolve;
    socket.onerror = reject;
  });
  let id = 0;
  const pending = new Map();
  socket.onmessage = event => {
    try {
      const message = JSON.parse(event.data);
      const callback = pending.get(message.id);
      if (!callback) return;
      pending.delete(message.id);
      if (message.error) callback.reject(new Error());
      else callback.resolve(message.result);
    } catch { socket.close(); }
  };
  socket.onclose = () => {
    for (const callback of pending.values()) callback.reject(new Error());
    pending.clear();
  };
  return {
    close: () => socket.close(),
    send(method, params = {}) {
      const requestId = ++id;
      return new Promise((resolve, reject) => {
        pending.set(requestId, { resolve, reject });
        socket.send(JSON.stringify({ id: requestId, method, params }));
      });
    },
  };
}

// Runs in an isolated world of one document. Native DOM functions cannot have
// been replaced by page scripts. Origin check and setter have no await between
// them. Navigation destroys the context rather than retargeting a different page.
function operation(request) {
  const url = new URL(location.href);
  if (!['http:', 'https:'].includes(url.protocol)) return { status: 'refused' };
  if (url.hostname.endsWith('.')) url.hostname = url.hostname.slice(0, -1);
  const origin = url.origin;
  if (request.value === null) return { origin };
  if (origin !== request.origin) return { status: 'origin_mismatch' };
  const elements = document.querySelectorAll(request.target);
  if (elements.length !== 1) return { status: 'refused' };
  const element = elements[0];
  if (!(element instanceof HTMLInputElement) || element.disabled || element.readOnly
      || !['text', 'email', 'password', 'tel', 'search', 'url', 'number'].includes(element.type)) {
    return { status: 'refused' };
  }
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
  setter.call(element, request.value);
  element.dispatchEvent(new Event('input', { bubbles: true }));
  element.dispatchEvent(new Event('change', { bubbles: true }));
  return { status: 'filled' };
}

async function execute(request) {
  const cdp = await connect(await pageSocket(request.session));
  try {
    const { frameTree } = await cdp.send('Page.getFrameTree');
    const { executionContextId } = await cdp.send('Page.createIsolatedWorld', {
      frameId: frameTree.frame.id, worldName: 'thevault-secret-fill',
    });
    const result = await cdp.send('Runtime.callFunctionOn', {
      executionContextId,
      functionDeclaration: operation.toString(),
      arguments: [{ value: request }],
      returnByValue: true,
    });
    if (result.exceptionDetails) throw new Error();
    const answer = result.result.value;
    if (request.value === null && typeof answer?.origin === 'string') return { origin: answer.origin };
    if (answer?.status === 'filled' || answer?.status === 'origin_mismatch') return { status: answer.status };
    throw new Error();
  } finally { cdp.close(); }
}

async function main() {
  const chunks = [];
  let size = 0;
  for await (const chunk of process.stdin) {
    size += chunk.length;
    if (size > 100000) throw new Error();
    chunks.push(chunk);
  }
  const request = JSON.parse(Buffer.concat(chunks).toString('utf8'));
  const answer = await execute(request);
  process.stdout.write(JSON.stringify(answer));
}

module.exports = { operation };
if (!module.parent) {
  const timeout = setTimeout(() => process.exit(1), 14000);
  main().catch(() => { process.exitCode = 1; }).finally(() => clearTimeout(timeout));
}
