---
id: '184'
title: AI draft tools for the Outbox and the send-without-review agent permission
plan: outbox
blocked_by:
- '180'
status: todo
verified: unknown
---

# AI draft tools for the Outbox and the send-without-review agent permission

Spec: /home/wil/dev/TheVault/.scratch/plans/outbox/SPEC.md, X9, X11. Acceptance: server tests against FakeAnthropic that draft_email, draft_sms and draft_event create review items, approved only with send_without_review, are offered to unattended runs, need write, and that the agent cannot then approve its own item; the Permissions tab group and switch (web test); the escalation check refuses giving the bypass. Targeted tests only.

## Worklog
