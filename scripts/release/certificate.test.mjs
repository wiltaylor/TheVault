import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFileSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

test('committed signer matches verified shipped alpha.37 APK', () => {
  const apk = '.scratch/tv-209/alpha37.apk';
  assert.equal(createHash('sha256').update(readFileSync(apk)).digest('hex'),
    'c234806dfe5f950f6af387d6e33f7adb24d94a410146868a5ec699cd54b551c3');
  const output = execFileSync(join(process.env.ANDROID_HOME, 'build-tools/36.0.0/apksigner'),
    ['verify', '--min-sdk-version', '33', '--print-certs', apk], {encoding: 'utf8'});
  const fingerprints = [...output.matchAll(/certificate SHA-256 digest: ([0-9a-f]+)/g)].map(match => match[1]);
  assert.deepEqual(fingerprints, [readFileSync('scripts/release/android-signer.sha256', 'utf8').trim()]);
});
