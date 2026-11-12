# Releasing TheVault locally

GitHub hosts source, tags, release downloads and ghcr packages. It runs no
TheVault build or test jobs. All nine workflows are inert manual notices.
There is no GitHub publishing fallback.

The release recipe runs only on the project's dedicated `release_target`.
Terrarium supplies the configured `docker_release` relay as `DOCKER_HOST`
(`TERRARIUM_DOCKER_HOST` is also accepted). The recipe never reads an SSH key.
Builds run in secret-free containers on that relay. Signing and registry/GitHub
publication run on the release client. Never run card tests on that client.

## Release identity and cut gate

Freeze `release/<X.Y.Z>` and run the full cut checks on its SHA, not on a moving
`next`. These are lead-owned runs, not per-card validation:

| Recipe | Target | Evidence |
|---|---|---|
| `just release::gate-dev` | thevault-dev, under /src/thevault-heavy.sh | just check and full web e2e |
| `just release::gate-android` | thevault-dev, under the heavy wrapper | validate-android and device fixture bundle |
| `bash scripts/nightly/device.sh` | thevault-android | full emulator checks from the transferred bundle |
| `just release::gate-box` | thevault-docker-vm | complete browser box image test |

Use one checkout per release. For example, after the lead prepares the checkout:

```sh
terrarium run thevault-dev --name release-dev --timeout 90m -- sh /src/thevault-heavy.sh sh -c 'cd /src/thevault-release && just release::gate-dev'
terrarium run thevault-dev --name release-android-build --timeout 90m -- sh /src/thevault-heavy.sh sh -c 'cd /src/thevault-release && just release::gate-android'
```

Do not start both commands concurrently against one writable checkout. Transfer
`.scratch/nightly/device.tar.gz` using `terrarium cp`. Follow the Android
bundle runbook in AGENTS.md, except a busy emulator lock at the cut must exit
75 (unfinished), never 0. The Android target has neither just nor pnpm.
The lead must obtain a real passing device report before recording the cut.
Upload the original reports with `terrarium test push --label version:<version>`.

Merge the frozen release into main with trailers in the final paragraph:

```text
pre-release: true
version: 0.1.14
tested-at-cut: <frozen-release-sha> <check-run-id> <e2e-run-id> <android-run-id> <device-run-id> <box-run-id>
```

The main merge commit cannot contain its own hash. The coordinator requires
its tree to equal the frozen commit's tree. The tag points to the main merge
commit, and the manifests record both SHAs and the shared tree. The lead attests
that the listed original test runs passed on that frozen tree. The script
does not query Terrarium to revalidate run IDs or rerun the cut suites.

`pre-release: true` selects the next alpha from the existing tags.
`release: true` selects the stable version and requires the operator's prior
approval. Never use both. `version: X.Y.Z` selects the base version. Without it,
the base is the patch after the highest stable tag. Alpha N runs from 1 to 98.
The coordinator fetches public tag history when a job's bundle has no tags.
An explicitly supplied version must match the trailer/tag calculation on the
first attempt. Resume keeps the selected version without recalculating alpha N.

## Delivery configuration and tools

META configures `release_recipe` to invoke:

```sh
just release::publish --delivery-config <delivery-snapshot.json> --state <durable-state-root>
```

The snapshot is the public routing portion of
`terrarium project delivery show --json`. Supply it as a recipe argument,
not a fixed path embedded in the script:

```json
{
  "delivery": {
    "release_target": "thevault-release-client",
    "docker_release": {
      "destination": "<configured SSH destination>",
      "port": 2251,
      "known_hosts": "<administrator-verified host key>"
    }
  }
}
```

No secret value belongs in that file. Only target, destination, port and pin are
kept in the journal. The daemon enforces the SSH pin and holds the relay key;
the recipe talks only to its job-scoped Docker socket. A changed routing
snapshot is rejected on resume.

The client needs Python 3.11+, git, bash, jq, Docker CLI/buildx, gh, crane,
Android build-tools (`zipalign`, `apksigner`, `aapt2`), cargo-tauri signer
and minisign on PATH. It needs writable tmpfs at /dev/shm. It does not need a
Rust compiler to build the application. The dev Dockerfile supplies the build
toolchain inside the relay container.

| Runtime environment | Purpose |
|---|---|
| DOCKER_HOST or TERRARIUM_DOCKER_HOST | daemon-provisioned Docker relay |
| GH_TOKEN | release-github, GitHub release access |
| GHCR_TOKEN | release-github, existing ghcr package write access |
| GHCR_USER | optional registry username, default wiltaylor |
| ANDROID_KEYSTORE_BASE64 | existing keystore, not a new key |
| ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS | existing Android signing identity |
| ANDROID_KEY_PASSWORD | optional, defaults to keystore password |
| ANDROID_SIGNER_SHA256 | public SHA256 fingerprint of the existing shipped APK signer |
| TAURI_SIGNING_PRIVATE_KEY, TAURI_SIGNING_PRIVATE_KEY_PASSWORD | existing updater signing key and password |

PATH, LANG, LC_ALL, TZ, RUSTUP_HOME and CARGO_HOME are allowed through to client
tools. HOME and tool configuration paths are replaced by private runtime paths.
Signing secrets never reach build containers. Registry auth and decoded
keystore material live in mode-0700 tmpfs directories removed on exit.
Passwords are environment references, not command-line values. Do not enable
shell tracing or dump the environment.

Missing signing prerequisites stop the run before signing, leaving the
unsigned build and its journal ready for the same-version resume. Do not
generate replacement signing keys. The Android fingerprint must match the
existing app. AppImage signatures are verified with the public updater key
embedded in the frozen source.

## Phase 1: prepare

`just release::publish` enters `scripts/release/release.py prepare`.

1. Freeze identity and routing under one release lock. Check cut evidence.
2. Build the dev image from the frozen source and create an isolated build
   container. Build unsigned arm64 and x86_64 release APKs, desktop bundles
   and startup smoke, AUR archive, both static CLIs and device fixtures.
3. Save the unsigned manifest and immutable signing inputs. Sign APKs at the
   custodian (zipalign first), verify signer and versionName/versionCode,
   then sign and verify the version-bound AppImage updater signature.
4. Recompute checksums. Build server and browser images on the relay from
   the same stamped source. Test served client hashes and the complete
   browser image behavior before exporting local image archives.
5. Create a draft release targeting the exact main SHA, upload all assets,
   and push only immutable version image tags. No edge, latest or
   major.minor channel moves yet.

The server and browser Dockerfiles still compile their own Rust/web components
inside relay containers. The unsigned build also uses the relay dev image.
No application binary executes on the key custodian.

Relay container names and local image tags all start
`thevault-release-<version>-<run-id>-`. Containers use their own writable
layers and explicit Docker copies, not host bind paths or named builders.
Cleanup removes only exact owned names. No prune or filter deletion is used.

The public asset manifest preserves alpha.37's 14 filenames: arm64 APK,
AppImage and signature, deb, rpm, AUR tarball, two CLIs, three checksum files,
compose.yaml, Caddyfile and env.example. The draft additionally carries the
x86_64 smoke APK, smoke-fixtures.tar.gz and release-manifest.json. These extra
assets remain attached when published.

The signed-manifest stage compares actual asset sizes against alpha.37 at
0.5x..2x and saves asset-comparison.json. A size outside that broad range stops
the release for review rather than publishing a likely stub or bloated package.

## Signed smoke handoff

Download the draft's signed x86_64 APK and smoke-fixtures.tar.gz with the lead's
authorized release access. Transfer them with `terrarium cp` into an owned
run directory on thevault-android. Never transfer credentials.

Extract the fixture archive into an empty owned directory, then run:

```sh
THEVAULT_APK=<absolute-signed-x86_64-apk> THEVAULT_RELEASE_SHA=<frozen-sha> bash scripts/release/device.sh
```

The script holds /home/agent/.thevault-emulator.lock from before boot until
cleanup. A busy lock exits 75. It uses port 5584, a private AVD/home and
PID/start-time ownership checks for cleanup. It records adb devices and
qemu PID, start time and command before and after. Emulator-5554 and the
`-avd thevault` process belong to someone else and must never be touched.
No process is killed by name or by a stale fixed PID.

The harness runs the release startup smoke and checks 1, 2, 3, 7, 8, 11, 17, 29.
Upload its original TAP report. Preserve
`.scratch/release-smoke/signed-smoke.json`, which binds the passing result to
the frozen SHA and signed x86_64 APK hash. This tests x86_64, not arm64 hardware.
A timeout requires inspecting the owned owner.json record and process start
time before PID-only cleanup.

## Phase 2: finish

After the lead has uploaded the smoke report, pass its original run ID and
the hash-bound sidecar back to a release job on the same main commit:

```sh
just release::finish --delivery-config <delivery-snapshot.json> --state <same-state-root> --smoke-evidence <signed-smoke.json> --smoke-run-id <test-run-id>
```

Finish verifies the local manifest, draft inventory and hashes, tag target,
signed APK hash and tested image digests. It then advances image channels and
publishes the draft. Pre-releases get version and edge only. Stable releases
also get major.minor and latest. The final step checks GitHub asset hashes,
tag SHA, release kind and registry channel digests.

Publication is not transactional across GitHub and ghcr. A failure partway
through finish can leave advanced image channels or a published draft.
Resume the same version and state, never delete the journal to recut it.
Existing mismatched assets or version tags are errors, not overwrite requests.

## Recovery and evidence

Keep the durable state root outside disposable job checkouts. It holds one
global lock, selected identity, per-version journal, source copy, manifests,
unsigned originals, signed assets, image archives and secret-free build/test
logs. Stages advance only after success. A failed stage is retried; completed
stages are not silently rerun. The signing stage restores its immutable
unsigned inputs before retrying a partial signing attempt.

Never edit a manifest to bless changed bytes. Resolve the cause or cut a new
authorized version. After verified publication, owned relay tags are removed.
Local archives and evidence remain for review and operator-controlled retention.
SIGKILL may leave exact run-owned containers; use the recorded run prefix to
inspect and remove those specific names only.

No step deploys production. Production upgrades remain META-owned with backup,
rollback and live checks. AUR output is a tarball, not an automated AUR push.

## Targeted development checks and rehearsals

Run on thevault-dev under the heavy wrapper:

```sh
node --test --test-reporter=tap scripts/release/local.test.mjs scripts/release/cache.test.mjs scripts/nightly/update-versions.test.mjs scripts/nightly/nightly.test.mjs
```

The local tests use Python's built-in unittest; no pytest install is needed.
Run actionlint against all nine inert workflows. Upload original TAP/XML output
with `terrarium test push --card TV-209`.

`just release::dry-run --delivery-config <fake-config.json> --plan-sha <40-hex-sha> --version 0.1.14-alpha.38`
prints every stage, target and expected public asset. It never builds, signs,
publishes or marks real stages complete. Unit command rehearsals use fake
tool responses, not real keys or registries. The alpha.37 fixture records
public asset metadata; comparison of that fixture is not a measurement of new
compiled artifact sizes. The lead's first real local pre-release is the
end-to-end proof, including actual builds, signing, smoke and publication.
