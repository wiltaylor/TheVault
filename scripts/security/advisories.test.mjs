import { test } from 'node:test';
import assert from 'node:assert/strict';
import { scan, publish, VERSION } from './advisories.mjs';

const vulnerability = { VulnerabilityID: 'CVE-2026-1234', PkgName: 'fixture', InstalledVersion: '1.0', FixedVersion: '2.0', Severity: 'HIGH', Description: 'secret must not escape' };
const result = (Type, vulnerabilities = []) => ({ Type, Vulnerabilities: vulnerabilities });
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
  const report = scan(fake(), '.', ['fixture.tar']);
  assert.deepEqual(report.coverage, ['runtime-image', 'rust', 'web']);
  assert.equal(report.findings.length, 1);
  assert.ok(!JSON.stringify(report).includes('secret'));
  assert.deepEqual(scan(fake(), '.', ['fixture.tar']), report);
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
  const report = scan(fake(), '.', ['fixture.tar']);
  publish(report, run);
  publish(report, run);
  assert.equal(cards.length, 1);
});
for (const [name, override] of [['tool/network failure', {status: 1}], ['malformed JSON', {stdout: 'bad'}], ['missing coverage', {stdout: '{"SchemaVersion":2,"Results":[]}'}]]) {
  test(name + ' is not green', () => assert.throws(() => scan(fake(override), '.', ['fixture.tar'])));
}
test('wrong version and missing runtime archive fail closed', () => {
  assert.throws(() => scan(() => ({status: 0, stdout: 'Version: other\n'}), '.', ['fixture.tar']));
  assert.throws(() => scan(fake(), '.', []));
});
test('backlog failure is not green', () => {
  assert.throws(() => publish(scan(fake(), '.', ['fixture.tar']), () => ({status: 1})));
});
