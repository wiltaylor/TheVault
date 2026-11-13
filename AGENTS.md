# Agents working on TheVault

Read `CLAUDE.md` for the source of truth, dev container and code layout. These test rules follow operator decisions D1-D4, 2026-11-12.

## Validation of a card

The flow is in_progress, review, merge_queue, accepted. There is no validating column and no `terrarium card validate` for card work.

Before review, push the card branch and run only targeted checks for the changed areas against that pushed commit on thevault-dev. Never run full `just validate`, `just check` or `just validate-android` per card or before every commit. Upload original test reports with `terrarium test push --card <id>`. Move to review with the branch, commit, commands, exit codes, pass counts and test run IDs. A failed check must be fixed or reported as blocked, not called a pass.

Use one clean checkout per card: `terrarium repo checkout thevault --target thevault-dev --branch <branch> --path /src/<card>`. Before every run, confirm the checked-out commit equals the pushed commit: read `.git/HEAD` and its ref file on the target, or use the HEAD printed by the checkout command. Reuse that path for every round. Write run output, reports, logs and build caches (`CARGO_TARGET_DIR`, web dependency copies) to `/src/<card>-out/`, never into the checkout. Do not create `/src/<card>-<round>` directories. All git commands run in the sandbox, never on a target. Branch from `origin/next` as `<card-id-lowercase>` and push only the card branch.

Never run a formatter inside the tested checkout. Format a copy, apply the change in the sandbox, then push.

A root crate dependency change must also update both app lockfiles, checked by `just ci::cargo-lock-check` (part of `just check`).

GitHub build/test CI is removed. The nine former workflows are deleted.
One manual sign-only workflow temporarily uses the existing repository
keys, as the operator requested. Releases use the local two-phase release job
documented in `docs/releasing.md`.

thevault-dev is the dev container itself. Run without `./dev`. Every build, test and lint there must use `/src/thevault-heavy.sh`, which holds one of two heavy slots and sets `CARGO_BUILD_JOBS=8`, `RUST_TEST_THREADS=8`, `nice -n10` and `ionice -c2 -n7` (MEM-160). For example:

```sh
terrarium run thevault-dev --name card-tests -- sh /src/thevault-heavy.sh sh -c 'cd /src/<card> && cargo test --locked -p <crate>'
```

Use `terrarium exec` with the same wrapper for short checks. For long runs, end the turn and resume on the result mail.

| area | targeted commands on thevault-dev |
|---|---|
| Rust | `cargo test --locked -p <crate>` for each changed crate and affected consumers. For a specific integration module: `cargo test --locked -p <crate> --test main <module>::`. Use package-scoped clippy and formatting checks for touched Rust. |
| Web | `pnpm -C web exec vitest run <test-file>` for affected tests, `pnpm -C web typecheck` for TypeScript changes, and `pnpm -C web exec playwright test <spec-file>` for affected browser flows. |
| Node harness | `npm ci --prefix apps/android/emulator --ignore-scripts --no-audit --no-fund`, then `node --test apps/android/emulator/<name>.test.mjs`. No emulator for harness-only unit tests. |
| Android | Test changed root crates (`embedded`, `sync`, `push`) with `cargo test --locked -p <crate>`. In `apps/android/src-tauri`, use `env -u CARGO_TARGET_DIR cargo test --locked -p <app-or-plugin-package> <test-filter>`. For Kotlin, in `apps/android/src-tauri/gen/android`, use `./gradlew --no-daemon testDebugUnitTest --tests '<class-or-method>'`. Prepare the server and host plugin library as in `just android-test` when those tests require them. Full `just validate-android` and emulator checks belong to the cut. |
| Scripts | `bash -n <changed-script.sh>` and the relevant tests: `node --test scripts/release/<name>.test.mjs`, `node --test scripts/nightly/nightly.test.mjs`, or `scripts/release/test.sh` for changed release shell behavior. |
| Docs only | Run the repository docs lint if available. There is currently no docs component in `just check`. Use `npx --yes markdownlint-cli@0.45.0 --disable MD013 MD024 MD033 MD041 -- <changed.md>` and report it separately, not as a full-suite pass. |

- Any change to `crates/api` routes or `crates/api/src/openapi.rs` runs the whole `cargo test --locked -p api --lib`. Add each new route to both review inventories: `safety_rest_read_route_inventory` in `crates/server/tests/agent_triggers.rs` and `crates/server/tests/browser_secrets_routes.txt`, with a review line on the card. A changed shipped default under `crates/writer/defaults` appends its new hash in `crates/writer/src/shipped.rs` (TV-229).
- Tests that build deep directory trees, use `/tmp` or look up the user name must also work in Docker: the default AppArmor profile refuses paths over about 8 KB (255-byte components fail at depth 31), `./dev` mounts `/tmp` as tmpfs, and CI or release containers can run as a uid with no passwd entry (TV-220, TV-223, TV-224).

A card is ready for review when its targeted checks exit 0. Explain which checks cover the changes and any assumptions. Review reads the diff and this evidence and runs no tests. Security cards still require a second reader (P8), who also reads without running tests.

### Per-worker Cargo output

Terrarium sets `CARGO_TARGET_DIR=/src/terrarium/thevault/<agent>/target` on
thevault-dev. This is outside the clean checkout and survives repeated checkouts
for that worker. Keep it set for root-workspace targeted tests and cut checks.
Android tests resolve the server and host plugin paths through `cargo metadata --locked`.
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

Release jobs run only on delivery `release_target`, with `docker_release`
provided as a daemon-owned Docker socket relay. Agents never test or exec on
the release client. The job builds in secret-free relay containers, obtains
signatures through the manual sign-only workflow by default, verifies them on
the client, uploads a draft and pushes version-only images. Custodian signing
requires THEVAULT_SIGNING_MODE=custodian. After the
lead runs signed x86_64 smoke on thevault-android, a second job publishes the
draft and advances image channels. The single recipe `just release::publish`
reads `release-phase: finish`, `release-version: <exact version>` and
`signed-smoke: <test run ID>` trailers for phase 2. The lead starts either job
with `terrarium job release <version-card> --commit <main-commit>`. No delivery
snapshot is passed to the script. The daemon serializes releases per project.
META can rebuild the client from its base, so private draft checkpoints, not
local files, preserve the unsigned build and journal across a rebuild.
Missing local keys do not stop default GitHub signing. The Android certificate fingerprint is
committed in `scripts/release/android-signer.sha256`.

The cut gate is lead-owned: `just release::gate-dev` and
`just release::gate-android` on thevault-dev under `/src/thevault-heavy.sh`,
the full transferred emulator bundle on thevault-android, and
`just release::gate-box` on thevault-docker-vm. The main merge's
`tested-at-cut: <frozen-sha> <run IDs>` trailer attests to the original passing
reports. The job requires the main and frozen release trees to match and tags
the main merge SHA. A busy emulator lock at a cut is unfinished (exit 75),
never a passing skip. Card checks remain targeted; do not run full cut gates
per card.

At the release cut only, a red full web e2e run receives one rerun of only its
failed tests (`playwright test --last-failed`). Passing reruns are reported as
flakes and do not block. A repeated failure blocks. Keep and upload both
original reports. `scripts/release/web_gate.py` enforces this, without retries
for missing results or infrastructure failures.

Signed smoke uses `bash scripts/release/device.sh`, port 5584, an owned AVD and
the shared emulator lock. Preserve emulator-5554 by identifying its qemu
command `-avd thevault`, recording PID and start time before and after, not by
assuming PID 19 or any other fixed PID. Cleanup is by owned PID/start identity.

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
