import {test} from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {updateVersions} from './update-versions.mjs';

for (const [version, expected] of [
  ['0.1.14', ['0.1.15', '0.1.16']],
  ['0.1.14-alpha.38', ['0.1.15', '0.1.16']],
  ['0.1.12', ['0.1.13', '0.1.14']],
]) {
  test(`update fixture versions for ${version}`, () => {
    assert.deepEqual(updateVersions(version), expected);
    const codes = updateVersions(version).map(update => Number(execFileSync(
      'bash', ['scripts/release/version-code.sh', update], {encoding: 'utf8'},
    ).trim()));
    assert.deepEqual(codes, version === '0.1.12' ? [101399, 101499] : [101599, 101699]);
  });
}
for (const version of ['0.1.NaN', '0.1.14-alpha.99', '0.1.14-beta.1', '01.1.14']) {
  test(`reject malformed update version ${version}`, () => {
    assert.throws(() => updateVersions(version));
  });
}
