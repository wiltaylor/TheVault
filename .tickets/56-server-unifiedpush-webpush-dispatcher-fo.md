---
id: '56'
title: 'Server: UnifiedPush WebPush dispatcher for notifications and commands'
plan: thevault-clients
blocked_by:
- '53'
status: done
verified: verified
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

### 2026-10-15T07:00:12Z — /work

Claimed.

### 2026-10-15T07:11:22Z — wil

Verified set to verified.

### 2026-10-15T07:11:22Z — wil

Status set to done.

### 2026-10-15T07:11:22Z — wil

Committed b077796 on ticket/thevault-clients-56, which fast-forwards over ticket/thevault-clients-54 (c2bab8d) for the state::Events bus: merge 54 first or together. New crate crates/push: webpush.rs (RFC 8291 encrypt/decrypt, appendix A vector checked) and a Dispatcher spawned in vault-server main. Payload {event,id} only; device.command goes only to its device (nothing publishes it yet: ticket 57 should call state.events().publish(LiveEvent::DeviceCommand{..})). Retries: 5 attempts, 2 s doubling, on network error/429/5xx; 404/410 -> State::drop_device_push (only if the endpoint is unchanged); other 4xx dropped. No VAPID signing (UnifiedPush does not need it). crates/embedded does not run the dispatcher: the in-process server is the device's own. S1 tests: crates/server/tests/push.rs; unit/seam tests: crates/push/tests/dispatch.rs.

### 2026-10-15T07:31:44Z — wil

Rebased onto main 96f3c62, ./dev just check passed, fast-forwarded main to 3a99c93. No Playwright specs added or changed, so no e2e run.
