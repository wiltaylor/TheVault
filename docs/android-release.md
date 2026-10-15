# Releasing the Android app

TheVault's Android app is not on Google Play. Each release is an APK signed with one permanent keystore and attached to a GitHub Release. Install it with `adb`, or let Obtainium track the releases (D10). The app needs Android 13 (API 33) or later (D14).

## The keystore

Android installs an update only when it is signed with the same key as the installed app. If the keystore is lost, no later release can update an installed app: the user must uninstall it, which deletes the phone's copy of the vault and anything not yet synced. Create the keystore once, keep it out of the repository, and back it up.

### Create the keystore

Create it on a trusted machine, in a folder outside any repository. `keytool` comes with any JDK. The dev container has one, and it mounts the folder you run it from:

```bash
mkdir -p ~/thevault-keys && cd ~/thevault-keys
~/dev/TheVault/dev keytool -genkeypair -v \
  -keystore thevault-release.jks -storetype PKCS12 \
  -alias thevault -keyalg RSA -keysize 4096 -validity 10000 \
  -dname "CN=TheVault"
```

`keytool` asks for the keystore password. A PKCS12 keystore uses the same password for the key. The key is valid for about 27 years.

Print the certificate's SHA-256 fingerprint and keep it with the backups. Use it to check that a release APK was signed with this key, and to register the app for developer verification:

```bash
~/dev/TheVault/dev keytool -list -v -keystore thevault-release.jks -alias thevault | grep SHA256
```

### Back up the keystore

1. Copy `thevault-release.jks` to two offline places, such as two USB drives kept apart.
2. Store the keystore password and the alias in your password manager.
3. Check one backup: run the `keytool -list` command above against the copy, with the stored password, and compare the fingerprint.

Never commit the keystore, and never put it inside the repository.

## Build a release locally

`just android-release` reads the keystore from these variables. `./dev` passes them into the container and mounts the keystore file read-only.

| Variable | Value |
|---|---|
| `THEVAULT_ANDROID_KEYSTORE` | The path to the keystore file |
| `THEVAULT_ANDROID_KEYSTORE_PASSWORD` | The keystore password |
| `THEVAULT_ANDROID_KEY_ALIAS` | The key's alias, `thevault` above |
| `THEVAULT_ANDROID_KEY_PASSWORD` | The key's password. Leave it unset when it is the keystore password |

```bash
export THEVAULT_ANDROID_KEYSTORE=~/thevault-keys/thevault-release.jks
export THEVAULT_ANDROID_KEY_ALIAS=thevault
read -rs THEVAULT_ANDROID_KEYSTORE_PASSWORD && export THEVAULT_ANDROID_KEYSTORE_PASSWORD
./dev just android-release
```

The recipe builds the release APK for aarch64, checks its signature with `apksigner verify`, and writes `target/android-release/TheVault_<version>_aarch64.apk` and `SHA256SUMS`. The version comes from `apps/android/src-tauri/tauri.conf.json`. When a variable is missing, the recipe stops and names it.

The release app's id is `dev.wiltaylor.thevault`. The debug app from `just android-build` is `dev.wiltaylor.thevault.debug`, so both can be installed on one phone.

## Publish a release

`.github/workflows/android-release.yml` runs on a `v*` tag. It builds the APK with `just android-release` in the dev container, and attaches the APK and `SHA256SUMS-android` to the tag's GitHub Release. It waits for the desktop workflow, `release.yml`, to create that release, and creates it itself only when the desktop workflow has not done so within 90 minutes.

Set these repository secrets once, under Settings > Secrets and variables > Actions:

| Secret | Value |
|---|---|
| `ANDROID_KEYSTORE_BASE64` | The keystore file, base64-encoded: `base64 -w0 thevault-release.jks` |
| `ANDROID_KEYSTORE_PASSWORD` | The keystore password |
| `ANDROID_KEY_ALIAS` | The key's alias |
| `ANDROID_KEY_PASSWORD` | The key's password, only when it differs from the keystore password |

To release:

1. Set `version` in `apps/android/src-tauri/tauri.conf.json` to the new version. Each release needs a higher version, because Android refuses an update whose version code is lower than the installed one.
2. Run the checks in `docs/android-manual-checks.md` on a phone.
3. Tag the commit as `v<version>`, for example `v0.2.0`, and push the tag. The workflow stops when the tag does not match the version.

## Install the app

### With adb

1. On the phone, turn on Developer options and USB debugging.
2. Connect the phone, then run:

   ```bash
   adb install -r TheVault_<version>_aarch64.apk
   ```

`-r` replaces an installed copy and keeps its data.

### With Obtainium

[Obtainium](https://github.com/ImranR98/Obtainium) installs apps from their GitHub Releases and checks for updates.

1. Install Obtainium.
2. Tap **Add App** and enter the URL of TheVault's GitHub repository.
3. In **Filter APKs by regular expression**, enter `TheVault_.*_aarch64\.apk`, then tap **Add**.
4. Tap **Install**. Allow Obtainium to install unknown apps when Android asks.

Obtainium shows an update when a newer release appears. Android installs it over the old copy only when the same keystore signed both.

### Allow restricted settings

Android 13 and later block some permissions for apps installed from outside an app store: SMS access on Android 15 and later, and notification access, which RCS capture needs. The app's capture setup walks through this. To do it by hand:

1. Open Android's Settings > Apps > TheVault.
2. Tap the three-dot menu at the top right, then **Allow restricted settings**, and confirm with your screen lock.
3. Go back to the app and grant the permission again.

The menu item appears only after Android has blocked a permission once. If it is missing, ask for the permission in the app first, then open App info.

## Developer verification from 2027

Google is rolling out developer verification for Android apps installed outside Google Play. Once it reaches a phone's region, a certified Android phone installs an app only when its package name is registered to a verified developer. Before the rollout, register a free Limited Distribution developer account, then register the package name `dev.wiltaylor.thevault` with the keystore's SHA-256 fingerprint. A Limited Distribution account covers installs on a small number of devices, which is enough for a single-user app. `adb` installs remain possible after the rollout.
