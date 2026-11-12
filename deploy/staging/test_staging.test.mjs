import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

for (const name of [
  'test_account_creates_key_without_returning_it',
  'test_account_reuses_named_account',
  'test_compose_has_no_network_and_one_capped_service',
  'test_recreate_has_fixed_project_and_no_global_cleanup',
  'test_decisions_binary_commands',
]) {
  test(name, () => {
    assert.ok(process.env.STAGING_DECISIONS_BIN, 'the fake binary must be tested, not skipped');
    const result = spawnSync('python3', ['-m', 'unittest', `test_staging.Contracts.${name}`], {
      cwd: fileURLToPath(new URL('.', import.meta.url)), encoding: 'utf8', timeout: 15000,
    });
    assert.equal(result.status, 0, result.stdout + result.stderr);
  });
}
