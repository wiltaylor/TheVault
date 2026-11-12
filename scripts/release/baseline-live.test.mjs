// Read-only comparison against GitHub metadata. Does not measure new builds.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

test('live alpha.37 public asset metadata agrees with the rehearsal baseline', async () => {
  const baseline = JSON.parse(readFileSync('scripts/release/alpha37-assets.json'));
  const response = await fetch(baseline.source, {signal: AbortSignal.timeout(30000)});
  assert.equal(response.status, 200);
  const release = await response.json();
  assert.equal(release.tag_name, `v${baseline.version}`);
  const sort = rows => rows.map(({name, size}) => ({name, size})).sort((a, b) => a.name.localeCompare(b.name));
  assert.deepEqual(sort(release.assets), sort(baseline.assets));
});
