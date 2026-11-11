#!/usr/bin/env bash
# Caller holds /home/agent/.thevault-emulator.lock until cleanup completes.
set -euo pipefail
export EMULATOR_PORT=5584 ANDROID_SERIAL=emulator-5584
export THEVAULT_AVD=thevault-nightly-5584 ANDROID_AVD_HOME="$PWD/.scratch/avd"
export THEVAULT_EMULATOR_OUT=.scratch/nightly
export THEVAULT_APK="$PWD/.scratch/nightly/apks/base.apk"
export THEVAULT_UPDATE_APK="$PWD/.scratch/nightly/apks/update1.apk"
export THEVAULT_UPDATE_APK2="$PWD/.scratch/nightly/apks/update2.apk"
source .scratch/nightly/updates.env
mkdir -p "$ANDROID_AVD_HOME"
exec > >(tee .scratch/nightly/device.log) 2>&1
snapshot() { adb devices; ps -eo pid,args | grep '[q]emu-system' || true; }
snapshot > .scratch/nightly/before.txt
cleanup() {
  status=$?
  trap - EXIT
  node --input-type=module <<'JS'
import {readFileSync} from 'node:fs';
import {processState} from './apps/android/emulator/reboot.mjs';
try {
  const r=JSON.parse(readFileSync('target/android-emulator/owner.json'));
  if (r.pid > 19 && r.serial === 'emulator-5584' && r.avd === process.env.THEVAULT_AVD && r.home === process.env.ANDROID_AVD_HOME && processState(r.pid)?.start === r.start) {
    process.kill(r.pid,'SIGTERM');
    for(let i=0; i<100 && processState(r.pid)?.start === r.start && !processState(r.pid)?.zombie; i++) await new Promise(resolve=>setTimeout(resolve,100));
    if(processState(r.pid)?.start === r.start && !processState(r.pid)?.zombie) process.kill(r.pid,'SIGKILL');
  }
} catch(e) { if(e.code !== 'ENOENT' && e.code !== 'ESRCH') throw e; }
JS
  snapshot > .scratch/nightly/after.txt
  exit "$status"
}
trap cleanup EXIT
if adb devices | grep -q '^emulator-5584[[:space:]]'; then
  echo 'port 5584 occupied, refusing to boot' >&2
  exit 1
fi
apps/android/emulator/boot.sh --wipe -writable-system
node apps/android/emulator/checks.mjs
