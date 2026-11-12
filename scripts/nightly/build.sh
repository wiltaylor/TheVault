#!/usr/bin/env bash
# Run only under /src/thevault-heavy.sh in a clean checkout.
set -euo pipefail
mkdir -p .scratch/nightly
exec > >(tee .scratch/nightly/build.log) 2>&1
export CARGO_TARGET_DIR="$PWD/target"
# Resolve checkout metadata by reading files, without running git on the target.
node --input-type=module <<'JS'
import {readFileSync, writeFileSync} from 'node:fs';
const head=readFileSync('.git/HEAD','utf8').trim();
let commit=head;
if(head.startsWith('ref: ')) {
  const ref=head.slice(5);
  try { commit=readFileSync(`.git/${ref}`,'utf8').trim(); }
  catch { commit=readFileSync('.git/packed-refs','utf8').split('\n').find(l=>l.endsWith(` ${ref}`))?.split(' ')[0]; }
}
if(!/^[a-f0-9]{40}$/.test(commit)) throw Error('Cannot identify checked-out commit');
writeFileSync('.scratch/nightly/commit',`${commit}\n`);
JS
just web-build
if [ "$1" = e2e ]; then
  cargo build -p server -p testkit --bins
  export PLAYWRIGHT_JUNIT_OUTPUT_FILE="$PWD/.scratch/nightly/e2e.xml"
  pnpm -C web exec playwright test --retries=0 --reporter=list,junit
  exit
fi
cargo build -p server -p cli --bins
just android-build x86_64
npm ci --prefix apps/android/emulator --ignore-scripts --no-audit --no-fund
apps/android/emulator/fake-messages/build.sh
mkdir -p .scratch/nightly/apks
mkdir -p .scratch/nightly/tools
tools=$(find "$ANDROID_HOME/build-tools" -mindepth 1 -maxdepth 1 -type d | sort -V | tail -1)
cp "$tools/aapt2" .scratch/nightly/tools/aapt2
cp apps/android/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk .scratch/nightly/apks/base.apk
# Build check 49's updates without stamping tracked files.
node --input-type=module <<'JS'
import {readFileSync, copyFileSync, writeFileSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
const conf = JSON.parse(readFileSync('apps/android/src-tauri/tauri.conf.json'));
const [major,minor,patch] = conf.version.split('.').map(Number);
let env = '';
for (const bump of [1,2]) {
  const version = `${major}.${minor}.${patch+bump}`;
  const code = Number(execFileSync('scripts/release/version-code.sh',[version],{encoding:'utf8'}).trim());
  const childEnv = {...process.env, CARGO_PROFILE_DEV_DEBUG:'0', CARGO_PROFILE_DEV_STRIP:'symbols', GRADLE_OPTS:'-Dorg.gradle.daemon=false'};
  delete childEnv.CARGO_TARGET_DIR;
  execFileSync('cargo',['tauri','android','build','--debug','--apk','--target','x86_64','--config',JSON.stringify({version,bundle:{android:{versionCode:code}}})],{cwd:'apps/android',env:childEnv,stdio:'inherit'});
  copyFileSync('apps/android/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk',`.scratch/nightly/apks/update${bump}.apk`);
  env += `export THEVAULT_UPDATE_VERSION${bump === 1 ? '' : '2'}=${version}\n`;
}
writeFileSync('.scratch/nightly/updates.env',env);
JS
mkdir -p .scratch/nightly/stage/apps/android/emulator/fake-messages
cp scripts/nightly/prebuilt-messages.sh .scratch/nightly/stage/apps/android/emulator/fake-messages/build.sh
chmod +x .scratch/nightly/stage/apps/android/emulator/fake-messages/build.sh
tar --exclude=apps/android/emulator/fake-messages/build.sh -cf .scratch/nightly/device.tar apps/android/emulator apps/android/src-tauri/tauri.conf.json docs/android-manual-checks.md scripts/nightly web/package.json web/node_modules web/dist target/debug/vault target/debug/vault-server target/android-emulator/fake-messages.apk .scratch/nightly/apks .scratch/nightly/tools .scratch/nightly/updates.env .scratch/nightly/commit
tar -rf .scratch/nightly/device.tar -C .scratch/nightly/stage apps/android/emulator/fake-messages/build.sh
gzip -f .scratch/nightly/device.tar
