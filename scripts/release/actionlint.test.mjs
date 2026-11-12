import {test} from 'node:test';
import {execFileSync} from 'node:child_process';

test('actionlint: all nine disabled workflows', () => {
  execFileSync(process.env.ACTIONLINT || 'actionlint', ['-shellcheck=', '-pyflakes='], {
    encoding: 'utf8', stdio: 'pipe',
  });
});
