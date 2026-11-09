# Agents working on TheVault

Read `CLAUDE.md` for everything else: the source of truth, the dev container, the code layout and the test rules. This file states how a card is validated (operator decisions P13, 2026-11-06, and H1-H5, 2026-11-07).

## Validation of a card

The flow is in_progress, review, merge_queue, accepted. There is no validating column and no `terrarium card validate` for card work.

Before you move a card to review, push the branch and run the recipes below yourself on thevault-dev against the pushed commit. Put the command, the commit and the pass count in the move comment. The lead's merge queue then merges the reviewed cards in one batch and runs the full suite on the result.

Run them on a clean checkout of the pushed commit, never on a working tree. Check it out with `terrarium repo checkout thevault --target thevault-dev --branch <branch> --path /src/<agent>`, then check out the branch at the pushed commit. Do not run `git -C /src/<agent> checkout <commit>` after the repo checkout: the target does not allow it.

GitHub CI runs only on pushes to `main` (the release train: desktop build, Android APK and smoke, images). It runs no tests on branches. So the recipes below cover more than CI.

thevault-dev is the dev container itself. Run the recipes there without `./dev`, with `CARGO_BUILD_JOBS` at 8 or less.

| recipe | target | cards | what it runs |
|---|---|---|---|
| `just validate` | thevault-dev | every card | `just check`: formatting, clippy with warnings denied, every Rust test, the web typecheck and vitest, the release scripts' tests |
| `just validate-android` | thevault-dev | cards that touch `apps/android`, `crates/embedded`, `crates/sync` or `crates/push`, or carry the label `android`, after `just validate` | `just android-check` (the Android lint of the embedded crates), then `just android-test` (the app's Rust tests and the Robolectric tests) |

A card is ready for review when each recipe it needs exits 0.

### Per-worker Cargo output

Terrarium sets `CARGO_TARGET_DIR=/src/terrarium/thevault/<agent>/target` on
thevault-dev. This is outside the clean checkout and survives repeated checkouts
for that worker. Keep it set for `just validate` and `just validate-android`.
Android tests resolve the server and host plugin paths through `cargo metadata`,
so both the configured output directory and Cargo's checkout-local defaults work.
`./dev` does not set this variable and keeps Cargo's default output directories.

Only one build or validation may write a worker's cache at a time. Never point
two agents at one writable target directory. Cargo separates host and Android
target-triple output and tracks each workspace's packages and dependencies.
Do not copy `apps/android/src-tauri/target` between checkout paths: Tauri build
scripts generate files in the source tree. Use a stable checkout path when
recreating a worker's clean checkout so those generated paths remain valid.
Keep `CARGO_BUILD_JOBS=8` or lower and run under `ionice -c3 nice -n19`.
The cache belongs to the worker and can be removed with that worker's leftovers.

## Merge queue

Only the lead merges. One merge job takes every card in the merge_queue column (`terrarium card merge A B C`), merges them onto `next` in order, runs `just check` once on the result, and pushes `next` when it is green. A card that conflicts goes back to ready. A red batch is split until the failing card is found. When a batch holds a card that needs `just validate-android`, the lead also runs that recipe on the merged `next`.

## Emulators on a shared machine

A machine can run emulators that are not yours, such as `emulator-5554` on thevault-android. Run `just android-emulator-checks` and `just android-release-smoke` there with `EMULATOR_PORT` set to a free console port (5556, 5558, ...) and your own AVD (`THEVAULT_AVD`, `ANDROID_AVD_HOME`). The recipes then set `ANDROID_SERIAL` to that emulator and kill only it. `boot.sh --wipe` refuses a port that already has an emulator.

## Only at release time

These do not run per card. They run on `release/<X.Y.Z>`:

- `just android-emulator-checks` and `just android-release-smoke` on thevault-android (about 1 hour, a shared box).
- The full end-to-end suite, `pnpm -C web e2e`.

thevault-android has no `just` and no `pnpm`, and takes no `terrarium repo checkout` (MEM-51). The release Android checks run in two places:

- On thevault-dev, build the x86_64 debug APK, `vault` and `vault-server`, `web/dist`, `fake-messages.apk`, and for check 49 the update APKs (`THEVAULT_UPDATE_APK`, `THEVAULT_UPDATE_APK2`). Copy them to thevault-android with `terrarium cp`.
- On thevault-android, run `node apps/android/emulator/checks.mjs` directly, with `THEVAULT_CHECKS` for a subset. Use your own AVD (`THEVAULT_AVD`, `ANDROID_AVD_HOME`), your own `EMULATOR_PORT`, and `ANDROID_SERIAL=emulator-<port>`.

`emulator-5554` and the AVD `thevault` belong to herdr: never touch them. Record `adb devices` and its qemu pid before and after the run, and kill only your own serial.
