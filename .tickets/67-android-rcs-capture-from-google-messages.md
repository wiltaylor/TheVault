---
id: '67'
title: Android RCS capture from Google Messages notifications (S5)
plan: thevault-clients
blocked_by:
- '66'
status: todo
verified: unknown
---

# Android RCS capture from Google Messages notifications (S5)

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC RCS capture (D13): a NotificationListenerService in the Kotlin plugin that parses Google Messages notifications (MessagingStyle extras: conversation title, sender person name, text, timestamp) into rcs_received events with partial = true and a stable device_ref (notification key + message timestamp); ignore SMS already captured from the provider (match on thread/time/text) to avoid duplicates; onboarding step for notification-listener access including Allow restricted settings. Record a few real-format sample notifications as test fixtures.

Acceptance criteria (S5):
- Fixture notifications produce the expected events; group chats map to one conversation; duplicates of provider SMS are dropped; non-Messages notifications are ignored.
- `./dev just android-test` passes.
Decisions: D13.

## Worklog
