#!/usr/bin/env bash
# Boot the headless Android 15 emulator the manual checks run on (docs/android-manual-checks.md),
# creating its AVD in the thevault-avd volume the first time. Run it through ./dev, which passes
# /dev/kvm through. It returns once Android has finished booting; the emulator keeps running in
# the background until `adb emu kill`, or until the container exits.
#
#   apps/android/emulator/boot.sh [--wipe] [extra emulator flags...]
#
# --wipe starts from a factory-fresh data partition. EMULATOR_PORT (default 5554) picks the
# console port; adb sees the emulator as emulator-<port>.
set -euo pipefail

avd=${THEVAULT_AVD:-thevault-api35}
image=${THEVAULT_AVD_IMAGE:-"system-images;android-35;google_apis;x86_64"}
port=${EMULATOR_PORT:-5554}
out=${THEVAULT_EMULATOR_LOGS:-target/android-emulator}
mkdir -p "$out" "${ANDROID_USER_HOME:-$HOME/.android}"

test -e /dev/kvm || { echo "/dev/kvm is missing: the emulator needs KVM; run this through ./dev on a host with KVM" >&2; exit 1; }
command -v emulator >/dev/null || { echo "emulator is missing; run this through ./dev" >&2; exit 1; }

if [ ! -f "${ANDROID_AVD_HOME:-$HOME/.android/avd}/$avd.ini" ]; then
  echo "creating AVD $avd from $image" >&2
  echo no | avdmanager create avd --force -n "$avd" -k "$image" -d pixel_6 >/dev/null
  config="${ANDROID_AVD_HOME:-$HOME/.android/avd}/$avd.avd/config.ini"
  {
    echo "hw.keyboard=yes"
    echo "hw.camera.back=virtualscene"
    echo "hw.camera.front=emulated"
    echo "disk.dataPartition.size=6G"
    echo "hw.ramSize=4096"
  } >> "$config"
fi

wipe=()
if [ "${1:-}" = "--wipe" ]; then
  wipe=(-wipe-data)
  shift
fi

serial="emulator-$port"
adb start-server >/dev/null
if adb devices | grep -q "^$serial"; then
  echo "$serial is already running" >&2
else
  # A container that died with the emulator running leaves the AVD's lock files behind, and the
  # next start refuses to run ("a snapshot operation is pending"). No emulator runs here now.
  rm -rf "${ANDROID_AVD_HOME:-$HOME/.android/avd}/$avd.avd/"*.lock
  # -no-snapshot: a cold boot each time, so no state survives in a snapshot between runs.
  nohup emulator -avd "$avd" -port "$port" -no-window -no-audio -no-boot-anim -no-snapshot \
    -gpu swiftshader_indirect -accel on "${wipe[@]}" "$@" > "$out/emulator.log" 2>&1 &
  echo "emulator pid $!, log $out/emulator.log" >&2
fi

export ANDROID_SERIAL=$serial
timeout 300 adb wait-for-device
for _ in $(seq 1 300); do
  if [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ]; then
    break
  fi
  sleep 1
done
[ "$(adb shell getprop sys.boot_completed | tr -d '\r')" = "1" ] || { echo "the emulator did not finish booting; see $out/emulator.log" >&2; exit 1; }

# Quiet, predictable UI for automation: no animations, the screen stays on while charging (the
# emulator always is), and no lock screen in the way.
adb logcat -G 16M >/dev/null 2>&1 || true
adb shell settings put global window_animation_scale 0
adb shell settings put global transition_animation_scale 0
adb shell settings put global animator_duration_scale 0
adb shell settings put global stay_on_while_plugged_in 7
adb shell locksettings set-disabled true >/dev/null 2>&1 || true
adb shell input keyevent KEYCODE_WAKEUP
adb shell wm dismiss-keyguard >/dev/null 2>&1 || true
echo "$serial booted: $(adb shell getprop ro.build.version.release | tr -d '\r') (API $(adb shell getprop ro.build.version.sdk | tr -d '\r'))" >&2
