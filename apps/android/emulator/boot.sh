#!/usr/bin/env bash
# Boot the headless Android 15 emulator the manual checks run on (docs/android-manual-checks.md),
# creating its AVD in the thevault-avd volume the first time. Run it through ./dev, which passes
# /dev/kvm through. It returns once Android has finished booting; the emulator keeps running in
# the background until `adb emu kill`, or until the container exits.
#
#   apps/android/emulator/boot.sh [--wipe] [extra emulator flags...]
#
# --wipe starts from a factory-fresh data partition, and refuses to run when an emulator already
# uses the port, which this run did not start. Without --wipe, one already running is used as it
# is. EMULATOR_PORT (default 5554) picks the console port; adb sees the emulator as
# emulator-<port>.
set -euo pipefail

avd=${THEVAULT_AVD:-thevault-api35}
image=${THEVAULT_AVD_IMAGE:-"system-images;android-35;google_apis;x86_64"}
port=${EMULATOR_PORT:-5554}
out=${THEVAULT_EMULATOR_LOGS:-target/android-emulator}
serial="emulator-$port"

wipe=()
if [ "${1:-}" = "--wipe" ]; then
  wipe=(-wipe-data)
  shift
fi

# A run that wipes needs an emulator it starts itself. One already on the port may be someone
# else's (another user's on a shared machine): never wipe, drive or kill it. Only listed here.
command -v adb >/dev/null || { echo "adb is missing; run this through ./dev" >&2; exit 1; }
adb start-server >/dev/null
running=no
if adb devices | grep -q "^$serial[[:space:]]"; then
  running=yes
fi
if [ "$running" = yes ] && [ "${#wipe[@]}" -gt 0 ]; then
  echo "$serial is already running, and --wipe needs an emulator this run starts: it may be another user's. Set EMULATOR_PORT to a free console port (5556, 5558, ...) and run again." >&2
  exit 1
fi
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

if [ "$running" = yes ]; then
  echo "$serial is already running" >&2
else
  # A container that died with the emulator running leaves the AVD's lock files behind, and the
  # next start refuses to run ("a snapshot operation is pending"). No emulator runs here now.
  rm -rf "${ANDROID_AVD_HOME:-$HOME/.android/avd}/$avd.avd/"*.lock

  # The emulator binds its modem simulator on [::1] first and on 127.0.0.1 only when that bind
  # fails, then tells QEMU to connect to the address it got. QEMU resolves ::1 with AI_ADDRCONFIG,
  # which fails when ::1 on lo is the only IPv6 address, as in a Docker container without IPv6
  # (TV-47): the modem is then dead, with no SIM and no radio. Here a listener holds [::1]:N during
  # the start and -modem-simulator-port N makes the emulator use that port, so it falls back to
  # 127.0.0.1:N, which QEMU resolves.
  modem=()
  if [ -r /proc/net/if_inet6 ] && grep -q '^00000000000000000000000000000001 ' /proc/net/if_inet6 \
    && ! grep -qv '^00000000000000000000000000000001 ' /proc/net/if_inet6 \
    && [[ " $* " != *" -modem-simulator-port "* ]]; then
    if command -v node >/dev/null; then
      : > "$out/modem-port"
      node -e '
        const net = require("node:net");
        // A port free on [::1] and on 127.0.0.1; the [::1] one stays held until this is killed.
        const pick = () => {
          const v6 = net.createServer().once("error", () => process.exit(1));
          v6.listen({ host: "::1", port: 0, ipv6Only: true }, () => {
            const port = v6.address().port;
            const v4 = net.createServer().once("error", () => v6.close(pick));
            v4.listen({ host: "127.0.0.1", port }, () => v4.close(() => console.log(port)));
          });
        };
        pick();
      ' > "$out/modem-port" &
      modem_holder=$!
      trap 'kill "$modem_holder" 2>/dev/null || true' EXIT
      for _ in $(seq 1 50); do
        [ -s "$out/modem-port" ] && break
        sleep 0.1
      done
      if [ -s "$out/modem-port" ]; then
        modem=(-modem-simulator-port "$(cat "$out/modem-port")")
        echo "::1 is the only IPv6 address: the modem uses 127.0.0.1:${modem[1]}" >&2
      else
        echo "could not hold a port on [::1]; the modem may fail" >&2
      fi
    else
      echo "::1 is the only IPv6 address and node is missing to work around it; the modem may fail" >&2
    fi
  fi

  # -no-snapshot: a cold boot each time, so no state survives in a snapshot between runs.
  nohup emulator -avd "$avd" -port "$port" -no-window -no-audio -no-boot-anim -no-snapshot \
    -gpu swiftshader_indirect -accel on "${modem[@]}" "${wipe[@]}" "$@" > "$out/emulator.log" 2>&1 &
  emulator_pid=$!
  echo "emulator pid $emulator_pid, log $out/emulator.log" >&2
  # Record only processes started here. adb disappearing does not establish that QEMU exited.
  EMULATOR_PID="$emulator_pid" EMULATOR_SERIAL="$serial" EMULATOR_AVD="$avd" \
    EMULATOR_AVD_HOME="${ANDROID_AVD_HOME:-$HOME/.android/avd}" node --input-type=module -e '
      import { writeFileSync } from "node:fs";
      import { processState } from "./apps/android/emulator/reboot.mjs";
      const pid = Number(process.env.EMULATOR_PID);
      const state = processState(pid);
      if (!state) throw new Error("emulator exited before ownership could be recorded");
      writeFileSync(process.argv[1], JSON.stringify({ pid, start: state.start,
        serial: process.env.EMULATOR_SERIAL, avd: process.env.EMULATOR_AVD,
        home: process.env.EMULATOR_AVD_HOME }));
    ' "$out/owner.json"
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
if [ -n "${modem_holder:-}" ]; then
  kill "$modem_holder" 2>/dev/null || true
fi

# The SMS and call checks need the modem. Without it the emulator boots, but adb emu sms send
# answers OK and Android gets nothing, so fail here instead.
if [ "$running" = no ] && grep -q "Unable to connect character device modem" "$out/emulator.log"; then
  echo "the emulator's modem did not connect: $(grep -m1 "Unable to connect character device modem" "$out/emulator.log")" >&2
  exit 1
fi
sim=
for _ in $(seq 1 60); do
  sim=$(adb shell getprop gsm.sim.state 2>/dev/null | tr -d '\r')
  case "$sim" in LOADED* | READY*) break ;; esac
  sleep 1
done
case "$sim" in
  LOADED* | READY*) ;;
  *) echo "the SIM is not ready (gsm.sim.state '$sim'): the modem does not work; see $out/emulator.log" >&2; exit 1 ;;
esac

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
