# Agents working on TheVault

Read `CLAUDE.md` for the source of truth, dev container and code layout. These test rules follow operator decisions D1-D4, 2026-11-12.

## Validation of a card

The flow is in_progress, review, merge_queue, accepted. There is no validating column and no `terrarium card validate` for card work.

Before review, push the card branch and run only targeted checks for the changed areas against that pushed commit on thevault-dev. Never run full `just validate`, `just check` or `just validate-android` per card or before every commit. Upload original test reports with `terrarium test push --card <id>`. Move to review with the branch, commit, commands, exit codes, pass counts and test run IDs. A failed check must be fixed or reported as blocked, not called a pass.

Use one clean checkout per card: `terrarium repo checkout thevault --target thevault-dev --branch <branch> --path /src/<card>`. Confirm the checkout identifies the pushed commit. Reuse that path and its cache for every round, keeping logs, reports and generated output inside it. Do not create `/src/<card>-<round>` directories. All git commands run in the sandbox, never on a target. Branch from `origin/next` as `<card-id-lowercase>` and push only the card branch.

GitHub CI runs only on pushes to `main` (the release train: desktop build, Android APK and smoke, images). It runs no tests on branches. So the recipes below cover more than CI.

thevault-dev is the dev container itself. Run without `./dev`. Every build, test and lint there must use `/src/thevault-heavy.sh`, which holds one of two heavy slots and sets `CARGO_BUILD_JOBS=8`, `RUST_TEST_THREADS=8`, `nice -n10` and `ionice -c2 -n7` (MEM-160). For example:

```sh
terrarium run thevault-dev --name card-tests -- sh /src/thevault-heavy.sh sh -c 'cd /src/<card> && cargo test -p <crate>'
```

Use `terrarium exec` with the same wrapper for short checks. For long runs, end the turn and resume on the result mail.

| area | targeted commands on thevault-dev |
|---|---|
| Rust | `cargo test -p <crate>` for each changed crate and affected consumers. For a specific integration module: `cargo test -p <crate> --test main <module>::`. Use package-scoped clippy and formatting checks for touched Rust. |
| Web | `pnpm -C web exec vitest run <test-file>` for affected tests, `pnpm -C web typecheck` for TypeScript changes, and `pnpm -C web exec playwright test <spec-file>` for affected browser flows. |
| Node harness | `npm ci --prefix apps/android/emulator --ignore-scripts --no-audit --no-fund`, then `node --test apps/android/emulator/<name>.test.mjs`. No emulator for harness-only unit tests. |
| Android | Test changed root crates (`embedded`, `sync`, `push`) with `cargo test -p <crate>`. In `apps/android/src-tauri`, use `env -u CARGO_TARGET_DIR cargo test -p <app-or-plugin-package> <test-filter>`. For Kotlin, in `apps/android/src-tauri/gen/android`, use `./gradlew --no-daemon testDebugUnitTest --tests '<class-or-method>'`. Prepare the server and host plugin library as in `just android-test` when those tests require them. Full `just validate-android` and emulator checks belong to the cut. |
| Scripts | `bash -n <changed-script.sh>` and the relevant tests: `node --test scripts/release/<name>.test.mjs`, `node --test scripts/nightly/nightly.test.mjs`, or `scripts/release/test.sh` for changed release shell behavior. |
| Docs only | Run the repository docs lint if available. There is currently no docs component in `just check`. Use `npx --yes markdownlint-cli@0.45.0 --disable MD013 MD024 MD033 MD041 -- <changed.md>` and report it separately, not as a full-suite pass. |

A card is ready for review when its targeted checks exit 0. Explain which checks cover the changes and any assumptions. Review reads the diff and this evidence and runs no tests. Security cards still require a second reader (P8), who also reads without running tests.

### Per-worker Cargo output

Terrarium sets `CARGO_TARGET_DIR=/src/terrarium/thevault/<agent>/target` on
thevault-dev. This is outside the clean checkout and survives repeated checkouts
for that worker. Keep it set for root-workspace targeted tests and cut checks.
Android tests resolve the server and host plugin paths through `cargo metadata`.
The server uses the configured output directory. Tauri app lint and Android build,
lint and test commands unset `CARGO_TARGET_DIR` to keep path-sensitive generated
output in each checkout's app workspace. Only root-workspace output is reused.
`./dev` does not set this variable and keeps Cargo's default output directories.

Only one build or validation may write a worker's cache at a time. Never point
two agents at one writable target directory. Cargo separates host and Android
target-triple output and tracks each workspace's packages and dependencies.
Do not copy `apps/android/src-tauri/target` between checkout paths: Tauri build
scripts generate files in the source tree. Never override the app isolation with
a shared Cargo configuration target-dir either.
Keep `CARGO_BUILD_JOBS=8` or lower and use the heavy wrapper above.
The cache belongs to the worker and can be removed with that worker's leftovers.

## Merge queue

Only the lead merges. One merge job takes every card in merge_queue (`terrarium card merge A B C`), merges onto `next` in order, pushes `next` and moves the cards to accepted. It runs no builds or tests, including Android, uid 1001, dashboard and Docker gates. A conflict still fails the batch and is split to identify the card, which goes back to ready.

## Production and leftovers

Never touch production. Upgrades go through meta/lead with a backup and a rollback plan. Do not treat a local check as production evidence.

After a card is merged or released, the lead may remove only that finished agent's own checkout, build output, emulator image, logs, temporary files and unused card Docker images (`docker rmi` without `-f`). Mail meta/lead each removed path or image and its size. Never delete shared data, volumes, shared images, another agent's files or anything an active card still uses. Other destructive work needs approval for the exact items. Stop only your own recorded PID or process group, never by name or pattern.

## Emulators on a shared machine

A machine can run emulators that are not yours, such as `emulator-5554` on thevault-android. For cut and nightly device checks use console port 5584, your own AVD (`THEVAULT_AVD`, `ANDROID_AVD_HOME`) and `ANDROID_SERIAL=emulator-5584`. A port conflict fails without touching its occupant. `boot.sh --wipe` refuses a port that already has an emulator. Stop only your emulator's recorded `owner.json` PID after verifying its start time.

## Release checks

For slow contact check 35, set `THEVAULT_CONTACT_DIAGNOSTIC=1` to save capture/job state at 30 s and wait without a foreground nudge (diagnosis only, normal timing checks stay unchanged).

At the cut, run the full checks once on the frozen `release/<X.Y.Z>`:

- D4a: `just check` on thevault-dev through the heavy lock.
- D4b: the full web end-to-end suite, `pnpm -C web e2e`, on thevault-dev through the heavy lock.
- D4c: `just validate-android` on thevault-dev through the heavy lock, then the full Android emulator checks and release smoke on thevault-android as described below.
- D4d: `just browser-image-test` on thevault-docker-vm, using throwaway containers and free ports.

A red cut goes back to the card that broke it (A3, A4). Keep the frozen version membership, fix on that release branch and retest the failed cut. Never recut from a newer `next` to hide a failure. Review and merge do not repeat these checks.

Cut rules:

- N2a: A blocker card runs only its own spec and targeted tests. After merging the fix, the lead runs the full suite once on the release branch.
- N2b: A red full run gets one diagnosis comment and one rerun of only the failed tests: `pnpm -C web exec playwright test --last-failed`. A test that passes on that rerun gets a flake card and does not block the cut. A test that fails again blocks it.
- N2c: Every full web e2e run uses one recipe, `just nightly-e2e`, which builds and finds its own binaries. Global setup reads `CARGO_TARGET_DIR` for both server and testkit binaries, falling back to `target` when unset. Explicit `VAULT_SERVER_BIN` and `TESTKIT_SEED_BIN` overrides still take precedence.

The cut's recipes retain their existing names: `just nightly-e2e` for the full web suite and `just nightly-emulator` to prepare the full emulator bundle. The nightly schedules are off. Run both recipes through the heavy lock on thevault-dev. The web recipe writes `.scratch/nightly/e2e.xml`, and the emulator recipe writes `.scratch/nightly/device.tar.gz` for transfer to thevault-android and execution with `bash scripts/nightly/device.sh` under the emulator lock. Upload the original reports even when a run fails.

thevault-android has no `just` and no `pnpm`, and takes no `terrarium repo checkout` (MEM-51). The release Android checks run in two places:

- On thevault-dev, build the x86_64 debug APK, `vault` and `vault-server`, `web/dist`, `fake-messages.apk`, and for check 49 the update APKs (`THEVAULT_UPDATE_APK`, `THEVAULT_UPDATE_APK2`). Copy them to thevault-android with `terrarium cp`.
- On thevault-android, run `node apps/android/emulator/checks.mjs` directly, with `THEVAULT_CHECKS` for a subset. Use your own AVD (`THEVAULT_AVD`, `ANDROID_AVD_HOME`), your own `EMULATOR_PORT`, and `ANDROID_SERIAL=emulator-<port>`.

`emulator-5554` and the AVD `thevault` belong to herdr: never touch them. Identify the protected qemu by its `-avd thevault` command, not a fixed PID (MEM-174). Record `adb devices` and `ps -eo pid,lstart,args` for that qemu before and after every run, including releases and nightlies. Never kill by serial or name, only your own verified owner PID.

Every project emulator run, **including releases**, must hold the fixed lock
`/home/agent/.thevault-emulator.lock` on thevault-android from before boot until
PID-only cleanup has completed. Do not wait for it in a nightly. Exit 0 with
`skipped: emulator busy`. A skipped release check is not a passed cut and must be rescheduled. Nightlies use console port 5584 only,
`ANDROID_SERIAL=emulator-5584`, and their own AVD/home, not the release AVD.
