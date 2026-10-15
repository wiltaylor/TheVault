---
id: '69'
title: 'Android notifications and UnifiedPush: mirror, cancel, swipe marks read, 15-minute
  fallback'
plan: thevault-clients
blocked_by:
- '68'
- '56'
status: done
verified: verified
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

### 2026-10-15T08:23:06Z — /work

Claimed.

### 2026-10-15T08:46:43Z — wil

Verified set to verified.

### 2026-10-15T08:46:43Z — wil

Status set to done.

### 2026-10-15T08:46:43Z — wil

Branch is based on ticket/thevault-clients-68 (3f97771), which was done but unmerged when 69 started. Merge 68 before 69. UnifiedPush uses connector 3.3.5, which does the RFC 8291 decryption itself; an interop test checks that its decrypter opens the RFC 8291 appendix A message that crates/push encrypts byte for byte. Notifications use native NotificationManager, not tauri-plugin-notification, so they work with no webview. The first reconcile notes already-unread notifications as seen without posting them, so pairing does not flood the shade; a push naming one still shows it. A tap from a cold start navigates with webView.post{loadUrl}; only manual check 21 covers whether that wins over the initial page load. device.command pushes are ignored here and belong to the device runtime. Tests: ./dev just android-test (NotificationMirrorTest 10/10, OnboardingActivityTest 4/4, plugin tests/notify.rs 2/2), ./dev just check green.

### 2026-10-15T08:56:52Z — wil

Merged to main as f2a8bb6, rebased onto 30a4ca0 after 68 (f1579e8) and 66 landed. Resolved conflicts in VaultDevicePlugin.kt, jni.rs, lib.rs and AndroidManifest.xml by keeping both sides. Onboarding still asks for POST_NOTIFICATIONS; 66's capture setup skips that step once it is granted. Rebased branch: just check, android-test and the plugin's fmt/clippy all green.
