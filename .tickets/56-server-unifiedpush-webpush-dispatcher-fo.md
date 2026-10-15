---
id: '56'
title: 'Server: UnifiedPush WebPush dispatcher for notifications and commands'
plan: thevault-clients
blocked_by:
- '53'
status: todo
verified: unknown
---

# Server: UnifiedPush WebPush dispatcher for notifications and commands

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement the push half of SPEC "Server: push and live events" (D4, D9): when a notification is created, read, dismissed or cleared, and when a device command is queued, send an RFC 8291 WebPush-encrypted message (ids only) to every device with a push endpoint; retry with backoff; drop endpoints that return 404/410. Use a maintained Rust WebPush crate if one fits the pinned toolchain (check licences), else implement aes128gcm encryption with ring/p256.

Acceptance criteria (S1):
- A fake push endpoint on port 0 receives a message that decrypts with the device's keys and names the notification id and event.
- 410 from the endpoint removes it from the device row.
- No push is attempted for devices without endpoints.
- `./dev just check` passes.
Decisions: D4, D9.

## Worklog
