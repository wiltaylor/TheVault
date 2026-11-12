import { readFileSync, existsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';
import { test } from 'node:test';

const read = (path) => readFileSync(path, 'utf8');

test('nightly shell scripts parse', () => {
  for (const file of ['build.sh', 'device.sh', 'prebuilt-messages.sh']) {
    execFileSync('bash', ['-n', `scripts/nightly/${file}`]);
  }
});

test('e2e runs without retries and writes JUnit in the checkout', () => {
  const script = read('scripts/nightly/build.sh');
  assert.match(script, /CARGO_TARGET_DIR="\$PWD\/target"/);
  assert.match(script, /PLAYWRIGHT_JUNIT_OUTPUT_FILE="\$PWD\/\.scratch\/nightly\/e2e.xml"/);
  assert.match(script, /--retries=0 --reporter=list,junit/);
});

test('device uses the isolated serial and AVD', () => {
  const script = read('scripts/nightly/device.sh');
  assert.match(script, /EMULATOR_PORT=5584 ANDROID_SERIAL=emulator-5584/);
  assert.match(script, /THEVAULT_AVD=thevault-nightly-5584/);
  assert.match(script, /THEVAULT_AVD_IMAGE='system-images;android-35;google_apis;x86_64'/);
  assert.match(script, /ANDROID_HOME=\/home\/agent\/sdk ANDROID_SDK_ROOT=\/home\/agent\/sdk/);
  assert.match(script, /ANDROID_AVD_HOME="\$PWD\/\.scratch\/avd"/);
  assert.match(script, /test -x \.scratch\/nightly\/tools\/aapt2/);
  assert.doesNotMatch(script, /emu kill|killall|pkill/);
});

test('cleanup requires recorded PID identity, start time, home and serial', () => {
  const script = read('scripts/nightly/device.sh');
  assert.match(script, /r.pid > 19/);
  assert.match(script, /r.serial === 'emulator-5584'/);
  assert.match(script, /r.home === process.env.ANDROID_AVD_HOME/);
  assert.match(script, /processState\(r.pid\)\?\.start === r.start/);
  assert.match(script, /trap cleanup EXIT/);
});

test('GitHub train is removed and local release requires the cut evidence', () => {
  assert.equal(existsSync('.github/workflows/release-train.yml'), false);
  assert.match(read('scripts/release/release.py'), /tested-at-cut must name the frozen SHA/);
  assert.match(read('.just/release/mod.just'), /gate-dev:\n\tjust check/);
});

test('device bundle includes the harness data files and badging tool', () => {
  const script = read('scripts/nightly/build.sh');
  assert.match(script, /apps\/android\/src-tauri\/tauri.conf.json docs\/android-manual-checks.md/);
  assert.match(script, /cp "\$tools\/aapt2"/);
});

test('schedule runbook takes a nonblocking device lock and forbids silent retries', () => {
  const docs = read('AGENTS.md');
  assert.match(docs, /flock -n 9/);
  assert.match(docs, /skipped: emulator busy/);
  assert.match(docs, /Never silently retry/);
  assert.match(docs, /diagnosis comment/);
});
