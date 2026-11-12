import {test} from 'node:test';
import assert from 'node:assert/strict';
import {updateVersions} from './update-versions.mjs';

for (const [version, expected] of [
  ['0.1.14', ['0.1.15', '0.1.16']],
  ['0.1.14-alpha.38', ['0.1.15', '0.1.16']],
  ['0.1.12', ['0.1.13', '0.1.14']],
]) {
  test(`update fixture versions for ${version}`, () => {
    assert.deepEqual(updateVersions(version), expected);
  });
}
for (const version of ['0.1.NaN', '0.1.14-alpha.99', '0.1.14-beta.1', '01.1.14']) {
  test(`reject malformed update version ${version}`, () => {
    assert.throws(() => updateVersions(version));
  });
}
