// These are plan/schema rehearsals, not compiled package size measurements.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, writeFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {execFileSync} from 'node:child_process';

const config = {delivery: {release_target: 'fake-release-client', docker_release: {
  destination: 'fake@relay', port: 2251, known_hosts: 'fake-pinned-host-key', key: 'not-a-real-secret',
}}};
const stages = ['preflight', 'cut', 'build', 'unsigned-manifest', 'sign', 'signed-manifest',
  'images', 'draft', 'push', 'smoke', 'publish', 'verify'];
const version = '0.1.14-alpha.38';
const sha = 'a'.repeat(40);
function plan() {
  const dir = mkdtempSync(join(tmpdir(), 'thevault-release-plan-'));
  try {
    writeFileSync(join(dir, 'delivery.json'), JSON.stringify(config));
    return JSON.parse(execFileSync('python3', ['scripts/release/release.py', 'plan',
      '--delivery-config', join(dir, 'delivery.json'), '--state', join(dir, 'state'),
      '--plan-sha', sha, '--version', version], {encoding: 'utf8'}));
  } finally { rmSync(dir, {recursive: true, force: true}); }
}
for (const stage of stages) {
  test(`fake-config dry-run stage: ${stage}`, () => {
    const output = plan();
    const entry = output.stages.find(row => row.stage === stage);
    assert.ok(entry);
    assert.equal(entry.sha, sha);
    assert.equal(entry.version, version);
    assert.equal(entry.dry_run, true);
    assert.ok(entry.target);
    assert.doesNotMatch(JSON.stringify(output), /not-a-real-secret/);
  });
}
test('alpha.37 public inventory matches planned public names with only version substitution', () => {
  const baseline = JSON.parse(readFileSync('scripts/release/alpha37-assets.json'));
  const output = plan();
  assert.deepEqual(Object.keys(output.assets).sort(), baseline.assets.map(row => row.name.replaceAll(baseline.version, version)).sort());
  assert.equal(baseline.assets.length, 14);
});
test('alpha.37 metadata sizes fit explicit baseline ranges, not new build measurements', () => {
  const baseline = JSON.parse(readFileSync('scripts/release/alpha37-assets.json'));
  const output = plan();
  for (const row of baseline.assets) {
    const role = output.assets[row.name.replaceAll(baseline.version, version)];
    const ranges = {android: [50e6, 200e6], desktop: [70e6, 250e6], deb: [15e6, 70e6], rpm: [15e6, 70e6],
      cli: [10e6, 50e6], aur: [500, 10000], 'updater-signature': [100, 2000], checksums: [50, 4000], compose: [100, 20000]};
    const [min, max] = ranges[role];
    assert.ok(row.size >= min && row.size <= max, `${row.name}: ${row.size}`);
  }
});
for (const path of ['scripts/release/build-unsigned.sh', 'scripts/release/device.sh', 'scripts/nightly/build.sh', 'docker/test-browser-image.sh']) {
  test(`shell syntax: ${path}`, () => execFileSync('bash', ['-n', path]));
}
test('signed smoke holds shared lock, fails busy, and never kills by name or serial', () => {
  const script = readFileSync('scripts/release/device.sh', 'utf8');
  assert.match(script, /exec 9>\/home\/agent\/\.thevault-emulator.lock/);
  assert.match(script, /flock -n 9 .*exit 75/);
  assert.match(script, /r.serial === 'emulator-5584'/);
  assert.match(script, /processState\(r.pid\)\?\.start === r.start/);
  assert.doesNotMatch(script, /pkill|killall|emu kill/);
});
test('just release module parses', () => {
  execFileSync('just', ['--list'], {encoding: 'utf8'});
});
