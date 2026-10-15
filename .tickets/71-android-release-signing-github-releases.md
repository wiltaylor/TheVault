---
id: '71'
title: 'Android release: signing, GitHub Releases workflow, Obtainium, manual checklist'
plan: thevault-clients
blocked_by:
- '68'
status: todo
verified: unknown
---

# Android release: signing, GitHub Releases workflow, Obtainium, manual checklist

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC Android distribution (D10): release build signed with a keystore whose path and passwords come from env/secrets (never committed); `just android-release` producing a signed APK; GitHub Actions release workflow uploading it (workflow file only - do not publish); docs: generating and backing up the keystore offline, installing with adb or Obtainium, Allow restricted settings steps, Limited Distribution developer account note for 2027; complete docs/android-manual-checks.md (S7) covering every SPEC S7 item.

Acceptance criteria:
- `./dev just android-release` with a throwaway test keystore produces an APK that `apksigner verify` accepts.
- The manual checklist covers every S7 item in SPEC.
Decisions: D10, D14.

## Worklog
