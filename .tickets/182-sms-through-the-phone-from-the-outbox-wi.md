---
id: '182'
title: SMS through the phone from the Outbox, with the result and the copy linked
plan: outbox
blocked_by:
- '180'
status: todo
verified: unknown
---

# SMS through the phone from the Outbox, with the result and the copy linked

Spec: /home/wil/dev/TheVault/.scratch/plans/outbox/SPEC.md, X6. Acceptance: server tests that approval queues send_sms with the outbox id and records the command id; sms_send off fails with the message; the command result route marks the item sent with device_ref or failed with the error; the captured sms_sent with that device_ref links result; an older app's result without device_ref still marks sent and the copy links by number, body and time. Android: Kotlin unit test (Robolectric, as android-test runs) for waiting on the sent intent and reading the row id; ./dev just android-test passes. Targeted tests only.

## Worklog
