# Releasing TheVault

A release is cut by a commit trailer on `main`. When the commit at the head of a push to `main` carries `pre-release: true`, GitHub Actions builds and publishes a pre-release such as `0.1.13-alpha.1`. When it carries `release: true`, it publishes a release such as `0.1.13`. A push without either trailer publishes nothing.

The workflow that does this is `.github/workflows/release-train.yml`. It builds the desktop packages, the static `vault` CLI, the Android APK and the server image from one commit, under one version, and publishes them together.

## Cut a release

1. Make sure everything you want in the release is merged into `main`.
2. Run the checks in [docs/android-manual-checks.md](android-manual-checks.md) on a phone. The train runs the Android smoke test itself and publishes nothing when it fails, so `./dev just android-release-smoke` beforehand is optional. It only finds a problem sooner.
3. Check which version the train will cut. Run `git fetch --tags`, then `scripts/release/version.sh pre-release` prints the next pre-release and `scripts/release/version.sh release` the next release. The script needs only git, so it runs on the host. `./dev just release-version pre-release` does the same in a main checkout, though not in a git worktree, whose repository the dev container cannot see.
4. Push an empty commit with the trailer to `main`. For a pre-release:

   ```sh
   git commit --allow-empty -m "Pre-release 0.1.13-alpha.1" -m "pre-release: true"
   git push origin main
   ```

   For a release, use `release: true` instead. Add a `version: X.Y.Z` trailer to set the version, as [Version rules](#version-rules) describes:

   ```sh
   git commit --allow-empty -m "Release 0.2.0" -m "release: true
   version: 0.2.0"
   ```

   Git reads trailers from the last paragraph of the message, so put them together after a blank line.
5. Watch the `release-train` run in the Actions tab. When it has finished, check that the GitHub Release lists the APK, the AppImage, the deb, the rpm, `thevault-bin-aur.tar.gz`, both `vault-<version>-linux-<arch>` builds, the `SHA256SUMS` files, `compose.yaml`, `Caddyfile` and `env.example`, and that `docker pull ghcr.io/wiltaylor/thevault:<version>` works.

The trailer can sit on any commit, not only an empty one. The train only reads the commit at the head of the push, so a trailer on an earlier commit of the same push does nothing.

To cut a release without a trailer commit, start `release-train` by hand from the Actions tab. Choose `pre-release` or `release` as `kind`, and fill in `version` only to set it. The run cuts the release from the head of `main`.

## What the train does

The train runs five jobs:

1. `version` reads the trailers of the head commit with `git log -1 --format='%(trailers:key=release,valueonly)'`, and the same for `pre-release` and `version`. It computes the version with `scripts/release/version.sh` and fails when that version's tag already exists, or when the commit carries both `release: true` and `pre-release: true`.
2. `desktop` calls `.github/workflows/release.yml`, which builds the AppImage with its updater signature, the deb, the rpm, the AUR package and the static `vault` CLI builds.
3. `android` calls `.github/workflows/android-release.yml` at the same time as `desktop`. It builds the signed aarch64 APK and checks its `versionName` and `versionCode`. Beside it, `android-smoke-reusable.yml` runs the [release smoke test](android-release.md#the-release-smoke-test) on an x86_64 build of the same commit and version.
4. `publish` runs only when `desktop` and `android`, the smoke test included, have passed. It creates the tag `v<version>` on the train's commit and the GitHub Release with every file. A pre-release is marked as a pre-release.
5. `image` calls `.github/workflows/image.yml`, which builds the server image with the release's APK, AppImage and CLI builds baked in, checks that it serves them, pushes it to `ghcr.io/wiltaylor/thevault`, and attaches the compose files to the release.

Each job writes the version into its own checkout before it builds, with `scripts/release/stamp.sh`. That sets `version` in `[workspace.package]` of `Cargo.toml` and in both app crates, the versions of those crates in the three `Cargo.lock` files, `version` in both `tauri.conf.json` files, and the Android `bundle.android.versionCode`. Nothing is committed, so `main` keeps the version it has. `./dev just release-stamp <version>` does the same to your checkout, and `git checkout .` undoes it.

GitHub Actions does not start workflows for a tag that a workflow pushes, so the train calls the build workflows itself rather than relying on the tag. Pushing a `v*` tag by hand starts nothing.

Trains run one at a time, so two never pick the same alpha number. A push without a trailer does not wait for a train in progress.

## Version rules

The base is the highest release tag, `vX.Y.Z` without a pre-release part. Pre-release tags never count as the base.

| Head commit carries | Version |
|---|---|
| `release: true` | The patch after the base: `0.1.13` after `v0.1.12` |
| `pre-release: true` | The patch after the base with the next alpha: `0.1.13-alpha.1`, then `0.1.13-alpha.2` |
| `release: true` and `version: 0.2.0` | `0.2.0` |
| `pre-release: true` and `version: 0.2.0` | `0.2.0-alpha.N`, N one more than the highest `v0.2.0-alpha.*` tag |

The alpha number counts from 1 to 98. A `version` trailer must be newer than the base. The train fails when the version's tag already exists, and it never moves a tag.

After `v0.1.13` is released, the next pre-release is `0.1.14-alpha.1`. A release always follows its own alphas, so `0.1.13-alpha.2` sorts before `0.1.13`.

## How each format writes a pre-release

Each package manager has its own rule for sorting a pre-release before its release. `scripts/release/package-version.sh` maps the version for each one:

| Where | `0.1.13-alpha.1` is written | `0.1.13` is written |
|---|---|---|
| Git tag, GitHub Release, file names, the app's own version | `0.1.13-alpha.1` | `0.1.13` |
| deb `Version` | `0.1.13~alpha.1` | `0.1.13` |
| rpm `Version` | `0.1.13~alpha.1` | `0.1.13` |
| AUR `pkgver` | `0.1.13alpha1` | `0.1.13` |
| Android `versionCode` | `101301` | `101399` |
| Server image tags | `0.1.13-alpha.1`, `edge` | `0.1.13`, `0.1`, `latest`, `edge` |

Tauri writes the semver into the deb, where dpkg reads `-alpha.1` as a Debian revision and sorts `0.1.13-alpha.1` after `0.1.13`. It cannot write the semver into an rpm at all, because an rpm version holds no `-`. For a pre-release, `just desktop-release` bundles the rpm as `0.1.13`, and `scripts/release/linux-packages.sh` rebuilds the deb and the rpm with the `~` version and names them by the semver. A release's packages are left as Tauri builds them.

The Android `versionCode` is `((major * 1000 + minor) * 1000 + patch) * 100 + N` for `alpha.N`, and `+ 99` for the release (`scripts/release/version-code.sh`). An alpha sorts after the release before it and before its own release. Every code is far above `1012`, the code Tauri gave `0.1.12`, the last release before this rule. `apps/android/src-tauri/tauri.conf.json` carries the code of its own version, so a local build follows the same rule.

`latest` and `<major>.<minor>` only ever name a release, so a server that runs `latest` never moves to a pre-release. `edge` names the newest build of either kind. Set `THEVAULT_VERSION=edge` in `.env` to follow the pre-releases. The GitHub Release of a pre-release is marked as one, so `releases/latest/download` in [docs/install.md](install.md) keeps naming the last release.

## Pre-releases and installed apps

An app updates to a pre-release only from a server that runs a pre-release. A server on `0.1.13` never moves its apps to a `0.1.14-alpha.1` it happens to publish. A server on `0.1.13-alpha.1` updates its apps to `0.1.13-alpha.1`, because the version gate pauses sync until the app matches the server ([The app updates before it syncs](desktop.md#the-app-updates-before-it-syncs)).

The rule holds in each place an app decides on an update:

- The version gate, `crates/sync/src/gate.rs`, which both apps run before each sync round (`may_take`).
- The Android update check, `AppUpdate.decide` in the device plugin, which reads `server_version` from `GET /api/clients`.
- The desktop updater's endpoint, `GET /api/clients/update/{target}/{arch}/{current_version}`, which offers a pre-release AppImage only when the server runs a pre-release, and the desktop's download offer in `apps/desktop/src-tauri/src/update.rs`.

A server before `0.1.13` sends no `server_version`, and the apps treat it as a release.

## Re-run a failed train

A failed train publishes nothing until `publish` runs, and `publish` only runs when every build has passed.

- When a build or the smoke test fails, fix the cause on `main` and cut a new release with a new trailer commit. If the failure was not in the code, such as a runner that ran out of disk, open the failed run and choose **Re-run failed jobs**. A re-run keeps the run's commit and version, because the `version` job does not run again.
- When `publish` fails, re-run the failed jobs. `publish` drafts the release before it publishes it, and a draft makes no tag, so a re-run finishes the draft an earlier run left.
- When `image` fails after `publish`, the release and its tag exist without the image and the compose files. Re-run the failed jobs, or start `image` by hand from the Actions tab with the tag, such as `v0.1.13`. A run by hand checks out the tag and writes the version from it.

Don't choose **Re-run all jobs** after `publish` has succeeded. The `version` job then finds the tag and fails.

## Secrets

The train uses the secrets the build workflows always used: `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` for the AppImage's updater signature ([docs/desktop.md](desktop.md#releasing)), and `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD` for the APK ([docs/android-release.md](android-release.md#publish-a-release)). It pushes the image with the run's own `GITHUB_TOKEN`.

## Test the scripts

`./dev just ci::release-test` runs `scripts/release/test.sh`, which `just check` runs too. It checks the version calculation against throwaway git repositories, the Android codes against the shipped `1012`, the order dpkg, rpm and pacman give the mapped package versions, the image tags, and stamping a copy of the checkout.
