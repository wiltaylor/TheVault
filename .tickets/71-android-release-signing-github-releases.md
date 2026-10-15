---
id: '71'
title: 'Android release: signing, GitHub Releases workflow, Obtainium, manual checklist'
plan: thevault-clients
blocked_by:
- '68'
status: done
verified: verified
---

# Android release: signing, GitHub Releases workflow, Obtainium, manual checklist

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC Android distribution (D10): release build signed with a keystore whose path and passwords come from env/secrets (never committed); `just android-release` producing a signed APK; GitHub Actions release workflow uploading it (workflow file only - do not publish); docs: generating and backing up the keystore offline, installing with adb or Obtainium, Allow restricted settings steps, Limited Distribution developer account note for 2027; complete docs/android-manual-checks.md (S7) covering every SPEC S7 item.

Acceptance criteria:
- `./dev just android-release` with a throwaway test keystore produces an APK that `apksigner verify` accepts.
- The manual checklist covers every S7 item in SPEC.
Decisions: D10, D14.

## Worklog

### 2026-10-15T08:38:29Z — /work

Claimed.

### 2026-10-15T08:55:35Z — wil

Verified set to verified.

### 2026-10-15T08:55:35Z — wil

Status set to done.

### 2026-10-15T08:55:35Z — wil

Committed b2fccae on ticket/thevault-clients-71. Criterion 1: with a throwaway PKCS12 keystore made by keytool outside the repo, ./dev just android-release wrote target/android-release/TheVault_0.1.0_aarch64.apk; apksigner verify -v printed 'Verifies' (v2 scheme true, 1 signer). Criterion 2: docs/android-manual-checks.md now has checks 16-39 and a coverage table mapping each of the nine S7 items to checks. Notes: (1) the Android workflow is its own file, android-release.yml, so it does not conflict with ticket 64's release.yml; but 64's workflow runs 'gh release create' with assets, which fails if the Android job created the release first. Whoever merges the second should make the desktop job upload when the release exists. (2) The first build failed because a Gradle client connected to another container's daemon over the shared /gradle volume and host network; android-release disables the daemon; filed ticket 75 for android-build. (3) Checks 29-30 (vault-to-phone contact edits) state the D16 behaviour; main has no default server-side trigger yet (ticket 70). (4) The R8-minified release APK was not installed on a phone; S7 is the check for that.

### 2026-10-15T09:09:06Z — wil

Merged to main as f9a4958 after rebasing onto 9f1e45e. Resolved a conflict with ticket 69's Notifications section: my checks are now 24-43 and the S7 coverage table cites 69's checks 16-20 for UnifiedPush. android-release.yml now waits up to 90 minutes for release.yml to create the tag's release, then uploads; this replaces the earlier note about the race. After the rebase, just check passed and android-release with the throwaway keystore again gave an APK that apksigner verifies.
