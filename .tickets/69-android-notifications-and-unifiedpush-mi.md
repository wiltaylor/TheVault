---
id: '69'
title: 'Android notifications and UnifiedPush: mirror, cancel, swipe marks read, 15-minute
  fallback'
plan: thevault-clients
blocked_by:
- '68'
- '56'
status: todo
verified: unknown
---

# Android notifications and UnifiedPush: mirror, cancel, swipe marks read, 15-minute fallback

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC Android notifications (D4, D9): our UnifiedPush plugin code on org.unifiedpush.android:connector (register, receive, decrypt WebPush, register endpoint with PUT /api/devices/{id}/push); on push, fetch the notification and show it with tauri-plugin-notification semantics (or native NotificationManager) using an id derived from the server notification id; read/dismiss/clear pushes cancel it; swipe-dismiss marks it read through the API; tap opens the note; without a distributor a WorkManager check every 15 minutes; POST_NOTIFICATIONS permission in onboarding.

Acceptance criteria (S5):
- A decrypted push for created shows a notification with the right id; read/dismiss/clear cancels it; a swipe calls the read endpoint; the fallback worker reconciles shown notifications with the server list.
- Add the timing check to docs/android-manual-checks.md (S7).
- `./dev just android-test` passes.
Decisions: D4, D9.

## Worklog
