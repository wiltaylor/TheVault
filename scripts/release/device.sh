#!/usr/bin/env bash
# Lead transfers smoke-fixtures.tar.gz and the signed x86_64 APK to an owned run
# directory, then runs this there. No build, signing key or publishing credential.
set -euo pipefail
: "${THEVAULT_APK:?set the signed x86_64 APK path}"
: "${THEVAULT_RELEASE_SHA:?set the frozen release SHA}"
exec 9>/home/agent/.thevault-emulator.lock
flock -n 9 || { echo 'release unfinished: emulator busy' >&2; exit 75; }
export ANDROID_HOME=/home/agent/sdk ANDROID_SDK_ROOT=/home/agent/sdk
export PATH="$ANDROID_HOME/emulator:$ANDROID_HOME/platform-tools:$ANDROID_HOME/cmdline-tools/latest/bin:$PATH"
export EMULATOR_PORT=5584 ANDROID_SERIAL=emulator-5584
export THEVAULT_AVD=thevault-release-5584 ANDROID_AVD_HOME="$PWD/.scratch/release-avd"
export THEVAULT_AVD_IMAGE='system-images;android-35;google_apis;x86_64'
export THEVAULT_SMOKE=1 THEVAULT_CHECKS=1,2,3,7,8,11,17,29 THEVAULT_CHECKS_ONLY=1
export THEVAULT_EMULATOR_OUT="$PWD/.scratch/release-smoke"
mkdir -p "$THEVAULT_EMULATOR_OUT" "$ANDROID_AVD_HOME"
tools=$(find "$ANDROID_HOME/build-tools" -mindepth 1 -maxdepth 1 -type d | sort -V | tail -1)
export PATH="$tools:$PATH"
"$tools/apksigner" verify --min-sdk-version 33 "$THEVAULT_APK"
snapshot() { adb devices; ps -eo pid,lstart,args | grep '[q]emu-system' || true; }
snapshot > "$THEVAULT_EMULATOR_OUT/before.txt"
cleanup() {
  status=$?
  trap - EXIT
  node --input-type=module <<'JS'
import {readFileSync} from 'node:fs';
import {processState} from './apps/android/emulator/reboot.mjs';
try {
  const r=JSON.parse(readFileSync('target/android-emulator/owner.json'));
  if (r.pid > 1 && r.serial === 'emulator-5584' && r.avd === process.env.THEVAULT_AVD && r.home === process.env.ANDROID_AVD_HOME && processState(r.pid)?.start === r.start) {
    process.kill(r.pid,'SIGTERM');
    for(let i=0; i<100 && processState(r.pid)?.start === r.start && !processState(r.pid)?.zombie; i++) await new Promise(resolve=>setTimeout(resolve,100));
    if(processState(r.pid)?.start === r.start && !processState(r.pid)?.zombie) process.kill(r.pid,'SIGKILL');
  }
} catch(e) { if(e.code !== 'ENOENT' && e.code !== 'ESRCH') throw e; }
JS
  snapshot > "$THEVAULT_EMULATOR_OUT/after.txt"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
trap 'exit 129' HUP
if adb devices | grep -q '^emulator-5584[[:space:]]'; then
  echo 'port 5584 occupied, refusing to boot' >&2
  exit 1
fi
apps/android/emulator/boot.sh --wipe -writable-system
node apps/android/emulator/checks.mjs
node --input-type=module <<'JS'
import {readFileSync, writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
const out=process.env.THEVAULT_EMULATOR_OUT;
const report=JSON.parse(readFileSync(`${out}/smoke.json`));
if(report.status !== 'pass') throw Error('signed smoke did not pass');
const apk_sha256=createHash('sha256').update(readFileSync(process.env.THEVAULT_APK)).digest('hex');
writeFileSync(`${out}/signed-smoke.json`, JSON.stringify({sha:process.env.THEVAULT_RELEASE_SHA,apk_sha256,status:'pass'},null,2)+'\n');
JS
