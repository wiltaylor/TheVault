#!/usr/bin/env bash
# TEST ONLY. Build the Google Messages stand-in (see AndroidManifest.xml) with the SDK's build
# tools, no Gradle: aapt2, javac, d8, zipalign and apksigner, signed with a throwaway key.
# Writes target/android-emulator/fake-messages.apk. Run it through ./dev.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
out="$repo/target/android-emulator/fake-messages"
apk="$repo/target/android-emulator/fake-messages.apk"
tools=$(ls -d "$ANDROID_HOME"/build-tools/* | sort -V | tail -n 1)
platform=$(ls -d "$ANDROID_HOME"/platforms/android-* | sort -V | tail -n 1)/android.jar

rm -rf "$out" && mkdir -p "$out/classes" "$out/dex"
"$tools/aapt2" link --manifest "$here/AndroidManifest.xml" -I "$platform" -o "$out/base.apk"
javac -source 11 -target 11 -nowarn -classpath "$platform" -d "$out/classes" $(find "$here/src" -name '*.java') 2>&1 | grep -v 'bootstrap classpath' || true
"$tools/d8" --min-api 33 --lib "$platform" --output "$out/dex" $(find "$out/classes" -name '*.class')
cp "$out/base.apk" "$out/unsigned.apk"
(cd "$out/dex" && "$tools/aapt" add "$out/unsigned.apk" classes.dex >/dev/null)
"$tools/zipalign" -f -p 4 "$out/unsigned.apk" "$out/aligned.apk"
key="$repo/target/android-emulator/fake-messages.keystore"
if [ ! -f "$key" ]; then
  keytool -genkeypair -keystore "$key" -storepass android -keypass android -alias test \
    -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=TheVault test only" >/dev/null 2>&1
fi
"$tools/apksigner" sign --ks "$key" --ks-pass pass:android --key-pass pass:android --out "$apk" "$out/aligned.apk"
echo "wrote $apk" >&2
