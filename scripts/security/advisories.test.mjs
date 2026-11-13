import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { scan, publish, VERSION } from './advisories.mjs';

const vulnerability = { VulnerabilityID: 'CVE-2026-1234', PkgName: 'fixture', InstalledVersion: '1.0', FixedVersion: '2.0', Severity: 'HIGH', Description: 'secret must not escape' };
const result = (Type, vulnerabilities = []) => ({ Type, Target: Type === 'cargo' ? 'Cargo.lock' : 'web/pnpm-lock.yaml', Vulnerabilities: vulnerabilities });
const images = ['server.tar', 'browser.tar'];
function fake(overrides = {}) {
  return (cmd, args) => {
    assert.equal(cmd, 'trivy');
    if (args[0] === '--version') return { status: 0, stdout: `Version: ${VERSION}\n` };
    assert.ok(args.includes('--offline-scan'));
    assert.ok(args.includes('--skip-db-update'));
    return { status: 0, stdout: JSON.stringify({ SchemaVersion: 2, Results: args[0] === 'fs' ? [result('cargo', [vulnerability, vulnerability]), result('pnpm')] : [result('debian')] }), ...overrides };
  };
}
test('complete coverage and sanitized deterministic deduplication', () => {
  const report = scan(fake(), '.', images);
  assert.deepEqual(report.coverage, ['runtime-image', 'rust', 'web']);
  assert.equal(report.findings.length, 1);
  assert.ok(!JSON.stringify(report).includes('secret'));
  assert.deepEqual(scan(fake(), '.', images), report);
});
test('repeated fixture publishes exactly one backlog item', () => {
  const cards = [];
  const run = (cmd, args) => {
    assert.equal(cmd, 'terrarium');
    if (args[1] === 'ls') return { status: 0, stdout: JSON.stringify(cards) };
    assert.ok(args.includes('backlog'));
    cards.push({ id: 'fixture' });
    return { status: 0 };
  };
  const report = scan(fake(), '.', images);
  publish(report, run);
  publish(report, run);
  assert.equal(cards.length, 1);
});
for (const [name, override] of [['tool/network failure', {status: 1}], ['malformed JSON', {stdout: 'bad'}], ['missing coverage', {stdout: '{"SchemaVersion":2,"Results":[]}'}]]) {
  test(name + ' is not green', () => assert.throws(() => scan(fake(override), '.', images)));
}
test('wrong version and missing runtime archive fail closed', () => {
  assert.throws(() => scan(() => ({status: 0, stdout: 'Version: other\n'}), '.', images));
  assert.throws(() => scan(fake(), '.', []));
  assert.throws(() => scan(fake(), '.', ['server.tar']));
  assert.throws(() => scan(fake(), '.', ['server.tar', 'server.tar']));
});
test('backlog failure is not green', () => {
  assert.throws(() => publish(scan(fake(), '.', images), () => ({status: 1})));
});
test('captured Terrarium card-list JSON skips existing advisories', {
  skip: !process.env.ADVISORY_CARD_LIST_JSON,
}, () => {
  const stdout = readFileSync(process.env.ADVISORY_CARD_LIST_JSON, 'utf8');
  const cards = JSON.parse(stdout);
  assert.ok(Array.isArray(cards));
  assert.ok(cards.length > 0);
  assert.equal(typeof cards[0].id, 'string');
  let lookups = 0;
  publish(scan(fake(), '.', images), (cmd, args) => {
    assert.equal(cmd, 'terrarium');
    assert.equal(args[1], 'ls', 'must not create a duplicate');
    lookups++;
    return { status: 0, stdout };
  });
  assert.equal(lookups, 2);
});
test('archived advisory prevents recreation', () => {
  publish(scan(fake(), '.', images), (cmd, args) => {
    assert.equal(args[1], 'ls');
    return { status: 0, stdout: JSON.stringify(args.includes('--archived') ? [{ id: 'archived' }] : []) };
  });
});
test('coverage requires exact lockfile targets and every image', () => {
  for (const missing of ['Cargo.lock', 'web/pnpm-lock.yaml', ...images]) {
    const run = fake();
    assert.throws(() => scan((cmd, args) => {
      const response = run(cmd, args);
      if (args[0] === '--version') return response;
      const report = JSON.parse(response.stdout);
      if (args.includes(missing)) report.Results = [];
      else for (const item of report.Results) if (item.Target === missing) item.Target = `vendor/${missing}`;
      return { status: 0, stdout: JSON.stringify(report) };
    }, '.', images));
  }
});
test('Debian tilde version is preserved and mentions are rejected', () => {
  const run = fake();
  const withPackage = name => (cmd, args) => {
    if (args[0] !== 'image') return run(cmd, args);
    return { status: 0, stdout: JSON.stringify({ SchemaVersion: 2, Results: [result('debian', [{ ...vulnerability, PkgName: name, InstalledVersion: '3.0.15-1~deb12u1' }])] }) };
  };
  assert.ok(scan(withPackage('openssl'), '.', images).findings.some(f => f.installed === '3.0.15-1~deb12u1'));
  assert.throws(() => scan(withPackage('@mention'), '.', images));
});
test('20 new findings create individual cards, 21 create one deduplicated summary', () => {
  const base = scan(fake(), '.', images).findings[0];
  for (const count of [20, 21]) {
    const cards = [];
    const report = { findings: Array.from({ length: count }, (_, i) => ({ ...base, key: String(i) })) };
    const run = (cmd, args) => {
      if (args[1] === 'ls') return { status: 0, stdout: JSON.stringify(cards.filter(c => c.includes(args[3]))) };
      assert.ok(args[2].includes('`'));
      assert.ok(args.at(-1).includes('advisory: `CVE-2026-1234`'));
      cards.push(args);
      return { status: 0 };
    };
    publish(report, run);
    publish(report, run);
    assert.equal(cards.length, count > 20 ? 1 : count);
  }
});
test('CLI failure emits only fixed diagnostics', () => {
  const result = spawnSync(process.execPath, [new URL('./advisories.mjs', import.meta.url).pathname], { encoding: 'utf8' });
  assert.equal(result.status, 1);
  assert.equal(result.stderr, 'ADVISORY_REPORT_FAILED\n');
  assert.deepEqual(JSON.parse(result.stdout), { schema: 1, status: 'failed' });
});
