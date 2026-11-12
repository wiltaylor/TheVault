import {test} from 'node:test';
import {execFileSync} from 'node:child_process';

test('actionlint: nine notice workflows and the manual sign-only workflow', () => {
  execFileSync(process.env.ACTIONLINT || 'actionlint', ['-shellcheck=', '-pyflakes='], {
    encoding: 'utf8', stdio: 'pipe',
  });
});
