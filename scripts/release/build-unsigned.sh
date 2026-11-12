#!/usr/bin/env bash
# Runs inside a fresh, named release container. No signing or publisher env.
set -euo pipefail
version=$1
export THEVAULT_NO_SIGNING_KEYS=1
unset THEVAULT_ANDROID_KEYSTORE THEVAULT_ANDROID_KEYSTORE_PASSWORD THEVAULT_ANDROID_KEY_ALIAS
unset THEVAULT_ANDROID_KEY_PASSWORD TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
mkdir -p "$HOME" /src/out/assets /src/out/smoke
bash scripts/release/stamp.sh "$version"
if [ "${THEVAULT_SIGNING_MODE:-github}" = none ]; then
  just cli-release
  cp target/cli-release/vault-*-linux-* /src/out/assets/
  cp target/cli-release/SHA256SUMS /src/out/assets/SHA256SUMS-cli
  cp deploy/compose.yaml deploy/Caddyfile /src/out/assets/
  cp deploy/.env.example /src/out/assets/env.example
  exit 0
fi
python3 scripts/release/gradle-wrapper.py apps/android/src-tauri/gen/android/gradle/wrapper/gradle-wrapper.properties .scratch/gradle-wrapper
just desktop-release-test
just aur-release
just cli-release
cp target/desktop-release/*.AppImage target/desktop-release/*.deb target/desktop-release/*.rpm target/desktop-release/thevault-bin-aur.tar.gz /src/out/assets/
cp target/cli-release/vault-*-linux-* /src/out/assets/
cp target/cli-release/SHA256SUMS /src/out/assets/SHA256SUMS-cli
cp target/desktop-release/SHA256SUMS /src/out/assets/SHA256SUMS
for abi in aarch64 x86_64; do
  # Only this container owns these outputs. Avoid selecting a stale other-ABI APK.
  rm -rf apps/android/src-tauri/gen/android/app/build/outputs/apk
  (cd apps/android && env -u CARGO_TARGET_DIR GRADLE_OPTS=-Dorg.gradle.daemon=false cargo tauri android build --apk --target "$abi")
  mapfile -t apks < <(find apps/android/src-tauri/gen/android/app/build/outputs/apk -name '*-release-unsigned.apk')
  [ "${#apks[@]}" = 1 ] || { printf 'expected one unsigned APK for %s\n' "$abi" >&2; exit 1; }
  dir=/src/out/assets
  if [ "$abi" = x86_64 ]; then dir=/src/out/smoke; fi
  cp "${apks[0]}" "$dir/TheVault_${version}_${abi}.apk"
done
(cd /src/out/assets && sha256sum *.apk > SHA256SUMS-android)
cp deploy/compose.yaml deploy/Caddyfile /src/out/assets/
cp deploy/.env.example /src/out/assets/env.example
# The signed smoke uses these host-side fixtures without rebuilding the APK.
cargo build -p server -p cli --bins
npm ci --prefix apps/android/emulator --ignore-scripts --no-audit --no-fund
apps/android/emulator/fake-messages/build.sh
mkdir -p /src/out/device
mkdir -p .scratch/release-device/apps/android/emulator/fake-messages
cp scripts/nightly/prebuilt-messages.sh .scratch/release-device/apps/android/emulator/fake-messages/build.sh
chmod +x .scratch/release-device/apps/android/emulator/fake-messages/build.sh
tar --exclude=apps/android/emulator/fake-messages/build.sh -cf /src/out/device/smoke-fixtures.tar apps/android/emulator apps/android/src-tauri/tauri.conf.json docs/android-manual-checks.md scripts/release/device.sh web/package.json web/node_modules web/dist target/debug/vault target/debug/vault-server target/android-emulator/fake-messages.apk
tar -rf /src/out/device/smoke-fixtures.tar -C .scratch/release-device apps/android/emulator/fake-messages/build.sh
gzip -f /src/out/device/smoke-fixtures.tar
