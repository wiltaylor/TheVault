import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

const source = readFileSync('deploy/compose.yaml', 'utf8');
const example = readFileSync('deploy/.env.example', 'utf8');
const hasCompose = spawnSync('docker', ['compose', 'version'], { encoding: 'utf8' }).status === 0;

// Scan production sources across all workspace crates, not tests or build.rs.
// Resolve literal reads and named constants, including the browser's local
// `var` closure. Clap arguments and unrelated VAULT strings are not env::var reads.
function runtimeKeys(root = 'crates') {
  const keys = new Set();
  function visit(dir) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = `${dir}/${entry.name}`;
      if (entry.isDirectory()) visit(path);
      else if (entry.name.endsWith('.rs')) {
        const rust = readFileSync(path, 'utf8');
        if (!/env\s*::\s*var(?:_os)?\s*\(/.test(rust)) continue;
        const constants = Object.fromEntries([...rust.matchAll(/const\s+(\w+)\s*:\s*&str\s*=\s*"([^"]+)"/g)]
          .map(m => [m[1], m[2]]));
        for (const match of rust.matchAll(/(?:env\s*::\s*var(?:_os)?|\bvar)\s*\(\s*(?:"([^"]+)"|([A-Z][A-Z0-9_]*))\s*\)/g)) {
          const key = match[1] ?? constants[match[2]];
          assert.ok(key, `unresolved environment constant ${match[2]} in ${path}`);
          if (!key.startsWith('VAULT_')) continue;
          if (!key.startsWith('VAULT_TEST_') && key !== 'VAULT_SERVER_BIN') keys.add(key);
        }
      }
    }
  }
  for (const crate of readdirSync(root, { withFileTypes: true })) {
    if (crate.isDirectory()) visit(`${root}/${crate.name}/src`);
  }
  return [...keys].sort();
}

test('server runtime environment keys are forwarded or deliberately allowlisted', t => {
  const keys = runtimeKeys();
  const allowed = JSON.parse(readFileSync('deploy/env-allowlist.json', 'utf8'));
  const compose = process.env.COMPOSE_ENV_CHECK_FILE
    ? readFileSync(process.env.COMPOSE_ENV_CHECK_FILE, 'utf8') : source;
  const server = compose.match(/^  thevault:\n([\s\S]*?)(?=^  [a-z]|^networks:)/m)?.[1];
  assert.ok(server, 'thevault service must exist');
  const environment = server.match(/^    environment:\n([\s\S]*?)(?=^    \S)/m)?.[1] ?? '';
  const forwarded = new Set([...environment.matchAll(/^      (VAULT_[A-Z0-9_]+):\s*\S/gm)].map(m => m[1]));
  t.diagnostic(`runtime keys: ${keys.join(', ')}`);
  for (const [key, reason] of Object.entries(allowed)) {
    assert.ok(keys.includes(key), `stale allowlist key: ${key}`);
    assert.ok(typeof reason === 'string' && reason.trim(), `${key} needs a reason`);
  }
  assert.deepEqual(keys.filter(key => !forwarded.has(key) && !Object.hasOwn(allowed, key)), [],
    'runtime keys missing from thevault environment (forward or allowlist with a reason)');
});

test('HTTPS example and container default remain explicit', () => {
  assert.match(source, /VAULT_SITE_HTTPS: \$\{VAULT_SITE_HTTPS:-0\}/);
  assert.match(example, /^VAULT_SITE_HTTPS=0$/m);
});

// Minimal renderer for the Compose operators used here. Docker config is preferred
// when installed. No daemon, images, containers or production access are needed.
function interpolate(text, env) {
  while (text.includes('${')) {
    const next = text.replace(/\$\{([A-Z_]+)(:?[-+])([^{}]*)\}/g, (_, key, op, fallback) => {
      const set = env[key] !== undefined && (!op.startsWith(':') || env[key] !== '');
      return op.endsWith('+') ? (set ? fallback : '') : (set ? env[key] : fallback);
    });
    assert.notEqual(next, text, 'unsupported interpolation');
    text = next;
  }
  return text;
}

function render(env) {
  if (hasCompose) {
    const result = spawnSync('docker', ['compose', '--env-file', '/dev/null', '-f',
      'deploy/compose.yaml', '--profile', 'https', '--profile', 'cloudflare',
      '--profile', 'browser', 'config', '--format', 'json'], {
      env: { PATH: process.env.PATH, HOME: process.env.HOME, ...env }, encoding: 'utf8',
    });
    assert.equal(result.status, 0, result.stderr);
    const config = JSON.parse(result.stdout);
    return {
      subnet: config.networks.proxy.ipam.config[0].subnet,
      range: config.networks.proxy.ipam.config[0].ip_range,
      ips: ['caddy', 'cloudflared', 'thevault'].map(name => config.services[name].networks.proxy.ipv4_address),
      trusted: config.services.thevault.environment.VAULT_TRUSTED_PROXIES,
    };
  }
  const rendered = interpolate(source, env);
  return {
    subnet: rendered.match(/- subnet: (.+)/)[1],
    range: rendered.match(/ip_range: (.+)/)[1],
    ips: [...rendered.matchAll(/ipv4_address: (.+)/g)].map(m => m[1]).slice(1)
      .concat(rendered.match(/ipv4_address: (.+)/)[1]),
    trusted: rendered.match(/VAULT_TRUSTED_PROXIES: *(.*)/)[1],
  };
}

test('defaults are outside Docker built-in pools and trust only the static proxies', t => {
  t.diagnostic(hasCompose ? 'docker compose config' : 'script interpolation fallback (Compose unavailable)');
  assert.deepEqual(render({}), {
    subnet: '10.141.0.0/24', range: '10.141.0.128/25',
    ips: ['10.141.0.2', '10.141.0.3', '10.141.0.4'],
    trusted: 'caddy=10.141.0.2,cloudflare=10.141.0.3',
  });
});

for (const prefix of ['172.30.141', '192.168.141']) {
  test(`existing ${prefix} network can be retained with matching trust`, () => {
    assert.deepEqual(render({
      THEVAULT_PROXY_SUBNET: `${prefix}.0/24`, THEVAULT_PROXY_IP_RANGE: `${prefix}.128/25`,
      THEVAULT_PROXY_CADDY_IP: `${prefix}.10`, THEVAULT_PROXY_CLOUDFLARED_IP: `${prefix}.11`,
      THEVAULT_PROXY_SERVER_IP: `${prefix}.12`,
    }), {
      subnet: `${prefix}.0/24`, range: `${prefix}.128/25`,
      ips: [`${prefix}.10`, `${prefix}.11`, `${prefix}.12`],
      trusted: `caddy=${prefix}.10,cloudflare=${prefix}.11`,
    });
  });
}

test('explicit trusted peers and explicit empty trust remain overrides', () => {
  for (const value of ['', 'caddy=10.0.0.8']) {
    assert.equal(render({ VAULT_TRUSTED_PROXIES: value }).trusted, value);
  }
});

test('example documents each network variable and agrees with defaults', () => {
  const env = Object.fromEntries([...example.matchAll(/^(THEVAULT_PROXY_[A-Z_]+)=(.*)$/gm)]
    .map(m => [m[1], m[2]]));
  const used = new Set([...source.matchAll(/\$\{(THEVAULT_PROXY_[A-Z_]+)/g)].map(m => m[1]));
  assert.deepEqual(new Set(Object.keys(env)), used);
  assert.deepEqual(render(env), render({}));
});
