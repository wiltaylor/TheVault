// Node's TAP reporter records the original result of each Python unittest.
import {test} from 'node:test';
import {readFileSync} from 'node:fs';
import {execFileSync} from 'node:child_process';

const cwd = new URL('.', import.meta.url);
const source = readFileSync(new URL('test_local.py', cwd), 'utf8');
for (const [, name] of source.matchAll(/^    def (test_\w+)\(self\):/gm)) {
  test(name, () => {
    execFileSync('python3', ['-m', 'unittest', `test_local.LocalReleaseTests.${name}`], {
      cwd, encoding: 'utf8', stdio: 'pipe',
    });
  });
}
