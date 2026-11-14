import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, existsSync, symlinkSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const cleanup = resolve('scripts/target-cleanup.sh');
const guard = resolve('scripts/target-build.sh');
function fixture() {
  const base = mkdtempSync('/src/tv-355-out/fixture-');
  const root = join(base, 'root');
  mkdirSync(root);
  const cards = join(base, 'cards');
  writeFileSync(cards, 'TV-1 done\nTV-2 accepted\nTV-3 in_progress\nTV-4 blocked\nTV-5 archived\nTV-6 review\nTV-7 merge_queue\n');
  const removed = ['tv-1', 'tv-1-out', 'tv-1-next', 'tv-1-baseline', 'tv-2', 'tv-5', 'rel-old'];
  const kept = ['tv-3', 'tv-4', 'tv-6', 'tv-7', 'tv-99', 'rel-current', 'rel-unknown'];
  for (const name of [...removed, ...kept]) mkdirSync(join(root, name));
  for (const name of ['terrarium', 'terrarium-cache', 'thevault-heavy.sh', 'thevault-heavy-lock']) writeFileSync(join(root, name), 'protected');
  const args = ['--cards', cards, '--current-release', 'rel-current', '--published-release', 'rel-old', '--published-release', 'rel-current'];
  const run = (...extra) => spawnSync('bash', [cleanup, ...args, ...extra], { encoding: 'utf8', env: { ...process.env, TARGET_CLEANUP_ROOT: root } });
  return { base, root, cards, removed, kept, run };
}
test('dry run lists exactly eligible trees and preserves everything', () => {
  const f = fixture(); const r = f.run('--dry-run');
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(r.stdout.split('\n').filter(x => x.startsWith('WOULD_REMOVE ')).map(x => x.slice(13)).sort(), f.removed.map(x => join(f.root, x)).sort());
  for (const name of f.removed) assert.ok(existsSync(join(f.root, name)));
});
test('apply removes exactly eligible trees', () => {
  const f = fixture(); const r = f.run('--apply'); assert.equal(r.status, 0, r.stderr);
  for (const name of f.removed) assert.ok(!existsSync(join(f.root, name)));
  for (const name of [...f.kept, 'terrarium', 'terrarium-cache', 'thevault-heavy.sh', 'thevault-heavy-lock']) assert.ok(existsSync(join(f.root, name)));
});
test('outside paths and symlinks refused before any deletion', () => {
  const f = fixture();
  assert.notEqual(f.run('--apply', '--published-release', '../outside').status, 0);
  mkdirSync(join(f.base, 'outside'));
  symlinkSync(join(f.base, 'outside'), join(f.root, 'tv-2-out'));
  assert.notEqual(f.run('--apply').status, 0);
  assert.ok(existsSync(join(f.root, 'tv-1')));
  assert.ok(existsSync(join(f.base, 'outside')));
});
test('duplicate states fail closed', () => {
  const f = fixture(); writeFileSync(f.cards, 'TV-1 done\nTV-1 blocked\n');
  assert.notEqual(f.run('--apply').status, 0);
  assert.ok(existsSync(join(f.root, 'tv-1')));
});
test('space floor refuses below requirement and allows above requirement', () => {
  const f = fixture();
  // Stub only df, so the test is independent of the target disk capacity.
  const bin = join(f.base, 'bin'); mkdirSync(bin);
  writeFileSync(join(bin, 'df'), '#!/bin/sh\nprintf "Filesystem 1024-blocks Used Available Capacity Mounted\\nfixture 999 0 1048576 0%% /fake\\n"\n', { mode: 0o755 });
  const run = floor => spawnSync('bash', [guard, 'sh', '-c', 'printf build-started'], { encoding: 'utf8', env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, TARGET_CLEANUP_ROOT: f.root, TARGET_MIN_FREE_GIB: floor } });
  const low = run('2'); assert.notEqual(low.status, 0); assert.match(low.stderr, /target-cleanup.sh/); assert.equal(low.stdout, '');
  const high = run('1'); assert.equal(high.status, 0); assert.equal(high.stdout, 'build-started');
});
