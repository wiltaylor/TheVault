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

## Release and nightly checks

For slow contact check 35, set `THEVAULT_CONTACT_DIAGNOSTIC=1` to save capture/job state at 30 s and wait without a foreground nudge (diagnosis only, normal timing checks stay unchanged).

At the cut, run the full checks once on the frozen `release/<X.Y.Z>`:

- D4a: `just check` on thevault-dev through the heavy lock.
- D4b: the full web end-to-end suite, `pnpm -C web e2e`, on thevault-dev through the heavy lock.
- D4c: `just validate-android` on thevault-dev through the heavy lock, then the full Android emulator checks and release smoke on thevault-android as described below.
- D4d: `just browser-image-test` on thevault-docker-vm, using throwaway containers and free ports.

A red cut goes back to the card that broke it (A3, A4). Keep the frozen version membership, fix on that release branch and retest the failed cut. Never recut from a newer `next` to hide a failure. Review and merge do not repeat these checks.

The following also have nightly coverage on `next`, not per-card gates:

- `just android-emulator-checks` and `just android-release-smoke` on thevault-android (about 1 hour, a shared box).
- The full end-to-end suite, `pnpm -C web e2e`.

thevault-android has no `just` and no `pnpm`, and takes no `terrarium repo checkout` (MEM-51). The release Android checks run in two places:

- On thevault-dev, build the x86_64 debug APK, `vault` and `vault-server`, `web/dist`, `fake-messages.apk`, and for check 49 the update APKs (`THEVAULT_UPDATE_APK`, `THEVAULT_UPDATE_APK2`). Copy them to thevault-android with `terrarium cp`.
- On thevault-android, run `node apps/android/emulator/checks.mjs` directly, with `THEVAULT_CHECKS` for a subset. Use your own AVD (`THEVAULT_AVD`, `ANDROID_AVD_HOME`), your own `EMULATOR_PORT`, and `ANDROID_SERIAL=emulator-<port>`.

`emulator-5554` and the AVD `thevault` belong to herdr: never touch them. Identify the protected qemu by its `-avd thevault` command, not a fixed PID (MEM-174). Record `adb devices` and `ps -eo pid,lstart,args` for that qemu before and after every run, including releases and nightlies. Never kill by serial or name, only your own verified owner PID.

Every project emulator run, **including releases**, must hold the fixed lock
`/home/agent/.thevault-emulator.lock` on thevault-android from before boot until
PID-only cleanup has completed. Do not wait for it in a nightly. Exit 0 with
`skipped: emulator busy`. A skipped release check is not a passed cut and must be rescheduled. Nightlies use console port 5584 only,
`ANDROID_SERIAL=emulator-5584`, and their own AVD/home, not the release AVD.

### Nightly schedule runbook (lead side)

The lead creates daily Terrarium schedules after merge: e2e at 16:00 UTC
(02:00 Brisbane), emulator at 18:00 UTC (04:00 Brisbane). GitHub runners cannot
reach these targets. The schedules mail the lead these steps. Targets have no
Terrarium CLI. Do not hold a turn open for a run. End the turn after `run`, then
continue on its result mail. No automatic retries.

N1. Before each night, recreate the **one fixed checkout** from origin/next.
Only remove these owned nightly paths after uploading the preceding evidence.
Never remove a directory while its run is active. E2e and emulator nights use
the same checkout sequentially, not simultaneously.

```sh
terrarium exec thevault-dev -- sh -c 'rm -rf /src/thevault-nightly'
terrarium repo checkout thevault --target thevault-dev --branch next --path /src/thevault-nightly
```

N2. At 16:00 UTC start e2e (45 minute timeout). All builds and tests on dev take
the heavy lock. The recipe overrides Cargo output to stay inside its checkout,
runs Playwright once with zero retries and writes original JUnit plus logs.

```sh
terrarium run thevault-dev --name nightly-e2e --timeout 45m -- sh /src/thevault-heavy.sh sh -c 'cd /src/thevault-nightly && just nightly-e2e'
```

On the result mail, even if red, copy and upload the original report. A missing
report is a failed infrastructure run, not a pass. Record the commit from the
`commit` file in the card and pass it as `--commit` when uploading.

```sh
mkdir -p .scratch/nightly
terrarium cp thevault-dev:/src/thevault-nightly/.scratch/nightly/commit .scratch/nightly/commit
terrarium cp thevault-dev:/src/thevault-nightly/.scratch/nightly/e2e.xml .scratch/nightly/e2e.xml
terrarium test push .scratch/nightly/e2e.xml --name nightly-e2e --branch next --label version:vnext
terrarium cp thevault-dev:/src/thevault-nightly/.scratch/nightly/build.log .scratch/nightly/e2e.log
```

Keep the printed test run ID. Attach logs (and retained Playwright traces for
failures) to the failure card before deleting output. After evidence upload:

```sh
terrarium exec thevault-dev -- sh -c 'rm -rf /src/thevault-nightly'
```

N3. At 18:00 UTC repeat N1, then prepare the emulator bundle on dev (90 minute
timeout). This builds host binaries, web, x86_64 debug and two update APKs, the
fake Messages APK and harness dependencies. No builds happen on android.
The bundle includes `aapt2`, the harness check titles and app config. Full
checks use the existing **rootable** Android 35 `google_apis;x86_64` image in
`/home/agent/sdk` (read only). The device script pins both `ANDROID_HOME` and
`ANDROID_SDK_ROOT` and the tool PATH there. Never use herdr's `/opt/android-sdk`:
its `google_apis_playstore` user image cannot remount `/product`.

```sh
terrarium run thevault-dev --name nightly-emulator-build --timeout 90m -- sh /src/thevault-heavy.sh sh -c 'cd /src/thevault-nightly && just nightly-emulator'
```

Only after a green build result, transfer the bundle and start the device run.
The shared lock covers extraction, boot, tests and cleanup. If skipped, do not
upload a stale report. The exclusive directory must not exist from an earlier
uncollected run. A port-5584 conflict fails without touching the occupant.

```sh
terrarium cp thevault-dev:/src/thevault-nightly/.scratch/nightly/device.tar.gz .scratch/nightly/device.tar.gz
terrarium exec thevault-android -- mkdir -p /home/agent/thevault-nightly
terrarium cp .scratch/nightly/device.tar.gz thevault-android:/home/agent/thevault-nightly/device.tar.gz
terrarium run thevault-android --name nightly-emulator --timeout 90m -- bash -euo pipefail -c 'exec 9>/home/agent/.thevault-emulator.lock; flock -n 9 || { echo "skipped: emulator busy"; exit 0; }; cd /home/agent/thevault-nightly; mkdir run; tar -xzf device.tar.gz -C run; cd run; bash scripts/nightly/device.sh'
```

On the device result mail, upload the original TAP, copy logs, commit and
before/after device and PID records. Do this for red runs too.

```sh
terrarium cp thevault-android:/home/agent/thevault-nightly/run/.scratch/nightly/results.tap .scratch/nightly/results.tap
terrarium test push .scratch/nightly/results.tap --name nightly-emulator --branch next --label version:vnext
terrarium exec thevault-android -- sh -c 'cd /home/agent/thevault-nightly/run && tar -czf ../evidence.tar.gz .scratch/nightly target/android-emulator'
terrarium cp thevault-android:/home/agent/thevault-nightly/evidence.tar.gz .scratch/nightly/emulator-evidence.tar.gz
```

Upload evidence with `terrarium artifact push` before cleanup. A timeout may
prevent the EXIT trap from finishing. Inspect the owner record and `/proc` start
time before killing only that owned PID, never kill by name or serial. Wait for
owned processes to exit before removing the AVD/run. Then remove owned output:

```sh
terrarium exec thevault-android -- sh -c 'rm -rf /home/agent/thevault-nightly'
terrarium exec thevault-dev -- sh -c 'rm -rf /src/thevault-nightly'
```

N4. A red result mails the lead with exit code and output tail. The lead creates
a backlog card naming the exact failing test, commit, test run IDs and logs.
Never silently retry. Before **any** rerun, add one diagnosis comment to that
card, consulting the state wiki flaky list (P6). Build/boot/upload failures also
need a card. A skip due to the emulator lock is not a test pass.

N5. The release train runs `just check` after dev-image, alongside desktop and
Android builds. `publish` depends on all three gates. The check job records its
elapsed seconds and exit code in the GitHub summary, with a 60 minute timeout.
TV-144 measured `just check` on thevault-dev at 539 seconds (8m59s), excluding
the heavy-lock wait. GitHub cold-run time can differ. The job runs alongside
builds, so added train wall time depends on which job finishes last.
