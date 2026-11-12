import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

const read = (path) => readFileSync(path, 'utf8');
const recipe = read('justfile').split('android-release-smoke:')[1]
  .split('\n\n# Run the app')[0].split('\n').slice(1)
  .map(line => line.replace(/^\t/, '')).join('\n');

test('smoke recipe shell syntax', () => {
  const result = spawnSync('bash', ['-n'], { input: recipe, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
});

test('smoke phases surround build, boot, checks and shutdown', () => {
  let previous = -1;
  for (const marker of ['phase=build', 'just web-build', 'cargo tauri android build',
    'THEVAULT_SMOKE_COMPILE_ONLY', 'apksigner', 'cargo build -p server',
    'phase=boot', 'boot.sh --wipe', 'phase=checks', 'node apps/android/emulator/checks.mjs',
    'phase=shutdown', 'emu kill', 'trap - EXIT']) {
    const index = recipe.indexOf(marker);
    assert.ok(index > previous, marker);
    previous = index;
  }
  assert.match(recipe, /THEVAULT_CHECKS=1,2,3,7,8,11,17,29/);
  assert.match(recipe, /sha256sum -- \*\.apk/);
});

test('cache paths exclude signing and runtime locations', () => {
  const action = read('.github/actions/release-cache/action.yml');
  const paths = [...action.matchAll(/^          ([^.\n][^\n]*|\.cache\/[^\n]*)$/gm)]
    .map(match => match[1]).filter(path => !path.startsWith('release-target-'));
  assert.ok(paths.length >= 10);
  for (const path of paths) {
    assert.match(path, /^(\.cache\/(cargo\/(registry|git)|pnpm\/store)|(?:apps\/(android|desktop)\/src-tauri\/)?target\/\*\*\/(deps|build|\.fingerprint))$/);
  }
  assert.match(action, /inputs.lane/);
  assert.match(action, /docker\/dev.Dockerfile/);
  assert.match(action, /github.sha/);
  assert.match(read('dev'), /THEVAULT_CARGO_CACHE:-thevault-cargo/);
  assert.match(read('dev'), /THEVAULT_PNPM_CACHE:-thevault-pnpm/);
});

test('only the manual signing workflow remains', () => {
  assert.deepEqual(readdirSync('.github/workflows').sort(), ['sign-only.yml']);
});

test('signing accepts manual hash-bound inputs with draft read access and no publication steps', () => {
  const workflow = read('.github/workflows/sign-only.yml');
  assert.match(workflow, /workflow_dispatch:/);
  assert.match(workflow, /contents: read/);
  assert.match(workflow, /sign:\n(?:    #[^\n]*\n)*    permissions:\n      contents: write/);
  assert.match(workflow, /persist-credentials: false/);
  assert.match(workflow, /--bundle-sha/);
  assert.match(workflow, /git merge-base --is-ancestor/);
  assert.doesNotMatch(workflow, /workflow_call:|push:|packages: write|cargo build|tauri build|gh release (create|edit|upload)|docker push/);
  assert.match(workflow, /actions\/upload-artifact@v6/);
});
