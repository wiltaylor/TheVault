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
Keep `CARGO_BUILD_JOBS=8` or lower and run under `ionice -c3 nice -n19`.
The cache belongs to the worker and can be removed with that worker's leftovers.

## Merge queue

Only the lead merges. One merge job takes every card in the merge_queue column (`terrarium card merge A B C`), merges them onto `next` in order, runs `just check` once on the result, and pushes `next` when it is green. A card that conflicts goes back to ready. A red batch is split until the failing card is found. When a batch holds a card that needs `just validate-android`, the lead also runs that recipe on the merged `next`.

## Emulators on a shared machine

A machine can run emulators that are not yours, such as `emulator-5554` on thevault-android. Run `just android-emulator-checks` and `just android-release-smoke` there with `EMULATOR_PORT` set to a free console port (5556, 5558, ...) and your own AVD (`THEVAULT_AVD`, `ANDROID_AVD_HOME`). The recipes then set `ANDROID_SERIAL` to that emulator and kill only it. `boot.sh --wipe` refuses a port that already has an emulator.

## Release and nightly checks

For slow contact check 35, set `THEVAULT_CONTACT_DIAGNOSTIC=1` to save capture/job state at 30 s and wait without a foreground nudge (diagnosis only, normal timing checks stay unchanged).

These do not run per card. They run on `release/<X.Y.Z>` and nightly on `next`:

- `just android-emulator-checks` and `just android-release-smoke` on thevault-android (about 1 hour, a shared box).
- The full end-to-end suite, `pnpm -C web e2e`.

thevault-android has no `just` and no `pnpm`, and takes no `terrarium repo checkout` (MEM-51). The release Android checks run in two places:

- On thevault-dev, build the x86_64 debug APK, `vault` and `vault-server`, `web/dist`, `fake-messages.apk`, and for check 49 the update APKs (`THEVAULT_UPDATE_APK`, `THEVAULT_UPDATE_APK2`). Copy them to thevault-android with `terrarium cp`.
- On thevault-android, run `node apps/android/emulator/checks.mjs` directly, with `THEVAULT_CHECKS` for a subset. Use your own AVD (`THEVAULT_AVD`, `ANDROID_AVD_HOME`), your own `EMULATOR_PORT`, and `ANDROID_SERIAL=emulator-<port>`.

`emulator-5554` and the AVD `thevault` belong to herdr: never touch them. Record `adb devices` and its qemu pid before and after the run, and kill only your own serial.

Every project emulator run, **including releases**, must hold the fixed lock
`/home/agent/.thevault-emulator.lock` on thevault-android from before boot until
PID-only cleanup has completed. Do not wait for it in a nightly. Exit 0 with
`skipped: emulator busy`. Never touch PID 19. Nightlies use console port 5584 only,
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
