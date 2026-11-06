# Agents working on TheVault

Read `CLAUDE.md` for everything else: the source of truth, the dev container, the code layout and the test rules. This file states how a card is validated (operator decision P13, 2026-11-06).

## Validation of a card

Each card is validated on a clean checkout of its pushed commit, never on a working tree. Check it out with `terrarium repo checkout thevault --target thevault-dev --branch <branch> --path /src/<agent>`, then the commit.

GitHub CI runs only on pushes to `main` (the release train: desktop build, Android APK and smoke, images). It runs no tests on branches. So the recipes below cover more than CI.

thevault-dev is the dev container itself. Run the recipes there without `./dev`, with `CARGO_BUILD_JOBS` at 8 or less.

| recipe | target | cards | what it runs |
|---|---|---|---|
| `just validate` | thevault-dev | every card | `just check`: formatting, clippy with warnings denied, every Rust test, the web typecheck and vitest, the release scripts' tests |
| `just validate-android` | thevault-dev | cards that touch `apps/android`, `crates/embedded`, `crates/sync` or `crates/push`, or carry the label `android`, after `just validate` | `just android-check` (the Android lint of the embedded crates), then `just android-test` (the app's Rust tests and the Robolectric tests) |

A card passes when each recipe it needs exits 0.

## Slots

At most N = 2 validation runs at once. More runs queue first in, first out.

## Emulators on a shared machine

A machine can run emulators that are not yours, such as `emulator-5554` on thevault-android. Run `just android-emulator-checks` and `just android-release-smoke` there with `EMULATOR_PORT` set to a free console port (5556, 5558, ...) and your own AVD (`THEVAULT_AVD`, `ANDROID_AVD_HOME`). The recipes then set `ANDROID_SERIAL` to that emulator and kill only it. `boot.sh --wipe` refuses a port that already has an emulator.

## Only at release time

These do not run per card. They run on `release/<X.Y.Z>`:

- `just android-emulator-checks` and `just android-release-smoke` on thevault-android (about 1 hour, a shared box).
- The full end-to-end suite, `pnpm -C web e2e`.
