# Releasing the Android app

TheVault's Android app is not on Google Play. Each release is an APK signed with one permanent keystore and attached to a GitHub Release. Install it with `adb`, or let Obtainium track the releases (D10). The app needs Android 13 (API 33) or later (D14).

## The keystore

Android installs an update only when it is signed with the same key as the installed app. If the keystore is lost, no later release can update an installed app: the user must uninstall it, which deletes the phone's copy of the vault and anything not yet synced. Create the keystore once, keep it out of the repository, and back it up.

### Where the keystore lives

The keystore and its passwords live outside the repository, in `~/.config/thevault/android/`, readable only by you:

| File | What it is |
|---|---|
| `~/.config/thevault/android/thevault-release.jks` | The PKCS12 keystore, with one RSA 4096 key under the alias `thevault` |
| `~/.config/thevault/android/keystore.env` | `export` lines for `THEVAULT_ANDROID_KEYSTORE`, `THEVAULT_ANDROID_KEYSTORE_PASSWORD`, `THEVAULT_ANDROID_KEY_ALIAS` and `THEVAULT_ANDROID_KEY_PASSWORD` |

`./dev` reads `keystore.env` by itself when it exists and `THEVAULT_ANDROID_KEYSTORE` is not already set, so `./dev just android-release` and `./dev just publish-clients` sign with the release key with no setup. Set `THEVAULT_NO_SIGNING_KEYS=1` to run `./dev` without it.

> **Back up `~/.config/thevault/android/` now** (both files), as [Back up the keystore](#back-up-the-keystore) says. It is the only copy. Without it, no phone can install an update over the app it has: each one must uninstall TheVault, losing its local vault and anything not yet synced, and install again.

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

### Release builds are not shrunk

The release build type turns R8 code shrinking and optimisation off (`isMinifyEnabled = false` in `apps/android/src-tauri/gen/android/app/build.gradle.kts`). Leave it off.

Release 0.1.1 was shrunk. R8 removed the constructor of `androidx.work.impl.WorkDatabase_Impl`, which Room creates by reflection, and the app crashed at start on every phone with a `NoSuchMethodException` from `androidx.startup.InitializationProvider`. Every test ran the debug build, which is never shrunk, so none of them saw it. With shrinking off, the release build runs the same code as the debug build the tests run; the APK is somewhat larger, which a sideloaded app can afford. The release build also allows cleartext http, like the debug build, because the home server is usually reached over plain http on a private network.

## The release smoke test

The release build differs from the debug build the other tests run: another package id, no WebView debugging, its own Gradle build type. So a release is tested as a release before any phone gets it:

```bash
./dev just android-release-smoke
```

The recipe:

1. Builds the release APK for x86_64, the emulator's ABI, into `target/android-release-smoke/`. Gradle signs it with the release key when the keystore is set up; otherwise the recipe signs it with the debug key, so the test needs no release keystore.
2. Builds the aarch64 release with `just android-release` from the same tree, when the release keystore is there. `THEVAULT_SMOKE_SKIP_AARCH64=1` skips this step, for a run where another job builds the aarch64 APK, as the release workflow does.
3. Boots the headless emulator from a wiped data partition, installs the x86_64 APK and launches it. The test fails when the app's process dies or restarts within 20 s, when logcat holds a `FATAL EXCEPTION` of `dev.wiltaylor.thevault`, or when the onboarding screen ("Connect TheVault to your server") does not show.
4. Runs checks 1, 2, 3, 7, 8, 11, 17 and 29 of [`android-manual-checks.md`](android-manual-checks.md) against a throwaway home server: first launch, a wrong and then the right password, a note from the server and an edit from the phone, the share target, a push notification and an SMS capture. A crash during the checks fails the test too.
5. When everything passes, writes `target/android-release/SMOKE_PASSED`, which lists the sha256 of the tested x86_64 APK and of the aarch64 APK built beside it.

Results go to `.scratch/emulator-release/`: `smoke.json`, `RESULTS.generated.md`, screenshots and logs. The test takes the emulator, so run no other emulator run at the same time.

A release APK cannot be driven with Playwright, because Tauri turns WebView debugging on only in debug builds. The release checks read the screen with `uiautomator dump` and the phone's vault through the emulator's root shell instead. Checks that drive the WebView with Playwright, such as 10, 13, 21, 42, 51 and 52 and the offline checks, need a debug APK. To run other checks against a release APK, set `THEVAULT_APK`, `THEVAULT_CHECKS` and `THEVAULT_CHECKS_ONLY=1` for `just android-emulator-checks`; the harness reads the package id from the APK.

### Publishing needs the smoke test

`just publish-clients` publishes a release APK only when its sha256 is in `target/android-release/SMOKE_PASSED`. When it builds the release itself, it runs `just android-release-smoke` rather than `just android-release`. A release APK that has not passed is not published, nor is a debug APK in its place; the desktop builds are still published, and the recipe exits with an error. A later `just android-release` deletes `SMOKE_PASSED`, so a rebuilt APK must pass again. This rule covers publishing from your machine, and still applies now that the release workflow runs the smoke test on GitHub: that run's result is not recorded in your `SMOKE_PASSED`.

In an emergency, `just publish-clients <dir> <build> skip` publishes the release APK without the test, with a warning. Do not use it for a normal release.

### The debug key

Debug APKs are signed with the Android Gradle plugin's debug keystore, `$ANDROID_USER_HOME/debug.keystore`. `./dev` sets `ANDROID_USER_HOME=/gradle/android-user`, in the `thevault-gradle` Docker volume, so every debug build on this machine is signed with the same key and a new debug APK installs over the last one. Before this, the keystore lived in the container's temporary home, so each build had a new key and no debug APK could update another. Deleting the `thevault-gradle` volume makes a new debug key.

### Switching keys: uninstall once

Android installs an update only over an app signed with the same key, and the debug and release apps are different apps (`dev.wiltaylor.thevault.debug` and `dev.wiltaylor.thevault`). So, once:

- A debug app built before the persistent debug key cannot be updated: uninstall it and install a new debug APK.
- To move from the debug app to the release app, install the release APK; it installs beside the debug app. Pair it, let it sync, then uninstall the debug app. The debug app never updates itself to a release build, and says so in its update status.

Sync everything first: uninstalling deletes the phone's local vault.

## Publish a release

`.github/workflows/android-release.yml` runs on a `v*` tag. It has three jobs:

1. `build` builds the aarch64 APK with `just android-release` in the dev container and keeps it, with its `SHA256SUMS`, as a workflow artifact.
2. `smoke` runs at the same time as `build`. It runs the [release smoke test](#the-release-smoke-test) on a GitHub-hosted runner with KVM, against an x86_64 release build of the same commit signed with the release key from the repository secrets. It keeps `.scratch/emulator-release/` as the `emulator-release` artifact, whether the test passed or failed. The steps live in `.github/workflows/android-smoke-reusable.yml`.
3. `attach` runs only when both `build` and `smoke` passed. It attaches the APK and `SHA256SUMS-android` to the tag's GitHub Release. It waits for the desktop workflow, `release.yml`, to create that release, and creates it itself only when the desktop workflow has not done so within 90 minutes.

When the smoke test fails, no APK is attached and the workflow fails, so `image.yml` publishes no image for the tag. Read the `emulator-release` artifact of the failed run, fix the app, and release a new version.

To run the same smoke test on GitHub without a tag, start `android-smoke` from the Actions tab. It signs the tested APK with the debug key unless you tick `release-key`.

The tag runs three workflows, in this order:

1. `release.yml` (desktop, AUR and the static `vault` CLI builds) and `android-release.yml` (the APK) run side by side. The desktop workflow creates the release, and the Android one adds the APK to it once the APK has passed the smoke test.
2. `image.yml` starts each time one of them succeeds. The run that finds the APK, the AppImage and both CLI builds on the release builds the server image with them baked in, checks that it serves them, pushes `ghcr.io/wiltaylor/thevault` as `<version>`, `<major>.<minor>` and `latest`, and attaches `compose.yaml`, `Caddyfile` and `env.example` to the release. The other run stops. To publish the image again, run `image.yml` by hand from the Actions tab with the tag.

The image workflow checks that the tag matches the workspace version in `Cargo.toml` and that both apps have that version, so set all three together. It uses the workflow's own `GITHUB_TOKEN` to push, and the package stays private while the repository is. `just image-release <assets>` runs the same build and check on your machine from a folder of the release's files, without pushing, as [docs/install.md](install.md#how-a-release-is-published) says.

Set these repository secrets once, under Settings > Secrets and variables > Actions:

| Secret | Value |
|---|---|
| `ANDROID_KEYSTORE_BASE64` | The keystore file, base64-encoded: `base64 -w0 thevault-release.jks` |
| `ANDROID_KEYSTORE_PASSWORD` | The keystore password |
| `ANDROID_KEY_ALIAS` | The key's alias |
| `ANDROID_KEY_PASSWORD` | The key's password, only when it differs from the keystore password |

To release:

1. Set `version` in `apps/android/src-tauri/tauri.conf.json` to the new version. Each release needs a higher version, because Android refuses an update whose version code is lower than the installed one.
2. Run the checks in `docs/android-manual-checks.md` on a phone. The release workflow runs the smoke test itself and publishes nothing when it fails, so running `./dev just android-release-smoke` before tagging is optional. It finds a problem before the tag rather than after, which saves a version number.
3. Set the same `version` in `apps/desktop/src-tauri/tauri.conf.json` and in `[workspace.package]` of `Cargo.toml`.
4. Tag the commit as `v<version>`, for example `v0.2.0`, and push the tag. Each workflow stops when the tag does not match the version.
5. When `image.yml` has finished, check the release lists the APK, the AppImage, `compose.yaml`, `Caddyfile` and `env.example`, and that `docker pull ghcr.io/wiltaylor/thevault:<version>` works.

### Releasing a format change

A release that adds an attribute to a type or a field, such as `labels` on a list field, ships type files that an older app has never seen. An app tolerates an attribute it does not know: it ignores it with a warning and keeps loading the type ([Attributes a reader does not know](vault-format.md#attributes-a-reader-does-not-know)). Apps from before that rule, 0.1.6 and earlier, refuse such a type file, and then refuse to open their vault on the next start.

From 0.1.9 the apps update themselves before they sync with a newer server ([The app updates before it syncs](#the-app-updates-before-it-syncs)), so the server goes first:

1. Restart the server on the new version, with the new app builds published in its clients folder: `./dev just publish-clients <clients dir>`.
2. Each app sees the newer server at its next sync round, pauses sync, updates itself and then syncs. Open the app on each phone to check that it shows the new version.

Apps before 0.1.10 also refuse an attribute inside a template, which 0.1.10 tolerates the same way. So a vault whose types use a tab's `when` ([Conditional tabs](vault-format.md#conditional-tabs)) needs apps at 0.1.10 or later. Apps from 0.1.9 update themselves before they sync, so the server-first order above brings them to 0.1.10 before they read such a type. Publish the 0.1.10 builds with the server before any type uses `when`.

Content block kinds are closed, so an app refuses a note holding a block kind it does not know. Apps at 0.1.10 and earlier do not know `toggle` and `admonition` ([toggle](vault-format.md#toggle), [admonition](vault-format.md#admonition)), so a vault whose notes use them needs apps from the release that added them, the first after 0.1.10, or later. Apps from 0.1.9 update themselves before they sync, so the server-first order above brings them up to date before they read such a note. Publish those builds with the server before anyone adds a toggle or a callout.

When the new builds cannot be published with the server, or a phone runs 0.1.8 or earlier, use the apps-first order instead:

1. Publish the new app first: `./dev just publish-clients <clients dir>`, with the old server still running. The old server announces the update.
2. Let the phones update, and open the app on each phone to check that it shows the new version.
3. Then restart the server on the new version. Its shipped defaults upgrade the vault's types, and the phones, already updated, read them.

## Install the app

### With adb

1. On the phone, turn on Developer options and USB debugging.
2. Connect the phone, then run:

   ```bash
   adb install -r TheVault_<version>_aarch64.apk
   ```

`-r` replaces an installed copy and keeps its data.

### From your server

When the server hands out the APK (see [`docs/clients.md`](clients.md)), open `https://<server>/apps` in the phone's browser, or scan the QR code on Settings > Devices, and tap **Download APK**. Open the downloaded file to install it. Allow the browser to install unknown apps when Android asks.

### Updates from your server

The app updates itself from the server it is paired with, once you allow it to install apps:

1. Publish a newer build on the server: `./dev just publish-clients <clients dir>` (see [`docs/clients.md`](clients.md)). The version in `apps/android/src-tauri/tauri.conf.json` must be higher than the installed one, and the release APK must have passed [the release smoke test](#the-release-smoke-test).
2. The server pushes `clients.updated` to the phone over UnifiedPush. Without push, the app checks when it opens and once a day.
3. The app reads `GET /api/clients`, and takes the APK for the phone's architecture when it is newer than the installed app and the same kind of build: a release app takes only release APKs, a debug app only debug APKs. It never installs the same or a lower version.
4. It downloads the APK from the server and checks the byte count against the list's `size` and the answer's `Content-Length`, and the SHA-256 against the list. A download that stops short is resumed where it stopped with an HTTP `Range` request (with `If-Range` naming the SHA-256, so a build replaced meanwhile is sent whole); one with the wrong bytes starts over. After 3 tries the app deletes the download and shows "TheVault update failed" with the SHA-256 and byte count it got, which Settings > General's About section shows too.
5. It installs the APK with an Android PackageInstaller session. The session asks for no user action, which the app's `UPDATE_PACKAGES_WITHOUT_USER_ACTION` permission allows on Android 12 and later for an app updating itself, so updates install silently, the first one after you installed the APK by hand too. If Android asks anyway, for example because another app store owns TheVault's updates, the phone shows "Update TheVault to <version>": tap it and confirm Android's dialog. When the new version starts, it shows "TheVault updated to <version>".

The check needs the phone online, but not idle or charging.

#### The app updates before it syncs

Before every sync round, in the foreground and in the background, the app compares the server's version (`server_version` in `GET /api/sync/info`) with its own (D27). The rule lives in the Rust sync core, `crates/sync/src/gate.rs`, which the Linux desktop app uses too:

- **Server newer, and an APK of the app's kind for the phone, at least as new as the server, published**: the round neither pushes nor pulls. The app runs the update at once, shows "Updating TheVault to <version> before syncing" in a notification and in Settings > Devices > This device, and the sync icon says sync waits. The new version syncs once it starts.
- **Server newer, and no such APK**: sync goes on, and This device warns that the server is newer than the app and publishes no matching app build.
- **An update that fails** (a download that stays wrong, the installer refusing it, no permission to install apps) keeps sync paused and is tried again after 1, then 4, then 16 minutes. After 3 failures to install one version, sync goes on with the warning, which names the last failure.
- **Server not newer**: sync runs as before.

#### Allow TheVault to install apps

Installing an APK needs Android's "Install unknown apps" permission for TheVault, granted once. The first time an update is ready without it, the app shows "TheVault <version> is ready". Tap it, tap **Allow updates**, and turn on **Allow from this source**. Then go back: the update starts.

To grant it by hand, open Settings > Apps > Special app access > Install unknown apps > TheVault. With adb: `adb shell appops set dev.wiltaylor.thevault REQUEST_INSTALL_PACKAGES allow`.

### With Obtainium

[Obtainium](https://github.com/ImranR98/Obtainium) installs apps from their GitHub Releases, or from a direct link, and checks for updates. The app updates itself from your server (above), so Obtainium is only needed to follow GitHub Releases, or for a phone that is not paired.

To follow the APK your own server hands out:

1. Install Obtainium.
2. Tap **Add App** and enter `https://<server>/api/clients/latest/apk`. This address always serves the newest APK the server publishes. Pick the **Direct APK Link** source if Obtainium does not choose it.
3. Tap **Add**, then **Install**.

Obtainium shows an update when the server publishes a new APK. The download needs no log-in.

To follow GitHub Releases instead:

1. Tap **Add App** and enter the URL of TheVault's GitHub repository.
2. In **Filter APKs by regular expression**, enter `TheVault_.*_aarch64\.apk`, then tap **Add**.
3. Tap **Install**. Allow Obtainium to install unknown apps when Android asks.

Obtainium shows an update when a newer release appears. Android installs it over the old copy only when the same keystore signed both.

### Allow restricted settings

Android 13 and later block some permissions for apps installed from outside an app store: SMS access on Android 15 and later, and notification access, which RCS capture needs. The app's capture setup walks through this. To do it by hand:

1. Open Android's Settings > Apps > TheVault.
2. Tap the three-dot menu at the top right, then **Allow restricted settings**, and confirm with your screen lock.
3. Go back to the app and grant the permission again.

The menu item appears only after Android has blocked a permission once. If it is missing, ask for the permission in the app first, then open App info.

## Developer verification from 2027

Google is rolling out developer verification for Android apps installed outside Google Play. Once it reaches a phone's region, a certified Android phone installs an app only when its package name is registered to a verified developer. Before the rollout, register a free Limited Distribution developer account, then register the package name `dev.wiltaylor.thevault` with the keystore's SHA-256 fingerprint. A Limited Distribution account covers installs on a small number of devices, which is enough for a single-user app. `adb` installs remain possible after the rollout.
