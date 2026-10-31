#!/bin/sh
# Start the Android emulator on the VNC server's display once it is up. The first start creates
# the AVD in /home/agent/.android, the home volume, so Google sign-ins, installed apps and their
# data persist across restarts. Every start is a cold boot with no snapshot saved, so stopping the
# container never leaves a half-written snapshot behind.
#
# ANDROID_AVD_NAME names the AVD, default thevault. ANDROID_RAM_MB sets its memory, default 4096.
set -eu

display=${DISPLAY:-:1}
socket="/tmp/.X11-unix/X${display#:}"
i=0
while [ ! -S "$socket" ]; do
  i=$((i + 1))
  if [ "$i" -gt 100 ]; then
    echo "thevault-emulator: display $display did not come up" >&2
    exit 1
  fi
  sleep 0.1
done

avd=${ANDROID_AVD_NAME:-thevault}
dir="$ANDROID_AVD_HOME/$avd.avd"
mkdir -p "$ANDROID_AVD_HOME"
if [ ! -f "$ANDROID_AVD_HOME/$avd.ini" ]; then
  echo "thevault-emulator: creating AVD $avd ($ANDROID_SYSTEM_IMAGE, $ANDROID_DEVICE)" >&2
  echo no | avdmanager create avd --name "$avd" --package "$ANDROID_SYSTEM_IMAGE" \
    --device "$ANDROID_DEVICE" --force > /dev/null
  cat >> "$dir/config.ini" << CONFIG
hw.keyboard=yes
hw.gpu.enabled=yes
hw.gpu.mode=swiftshader_indirect
disk.dataPartition.size=8G
CONFIG
fi
sed -i "/^hw.ramSize=/d" "$dir/config.ini"
echo "hw.ramSize=${ANDROID_RAM_MB:-4096}" >> "$dir/config.ini"

# A container killed mid-run leaves these behind, and the emulator then refuses the AVD.
rm -f "$dir"/*.lock

exec "$ANDROID_HOME/emulator/emulator" -avd "$avd" -gpu swiftshader_indirect -no-audio \
  -no-boot-anim -no-snapshot -no-metrics
