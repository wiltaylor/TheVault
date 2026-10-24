---
id: '192'
title: 'Activity log store and recording: every vault change and all AI activity,
  with the API and live events'
plan: activity
blocked_by: []
status: todo
verified: unknown
---

# Activity log store and recording: every vault change and all AI activity, with the API and live events

Spec: /home/wil/dev/TheVault/.scratch/plans/activity/SPEC.md, Y1, Y2, Y3, Y6 and Y7's API part. Acceptance: state tests (record, page with before, filters, retention by age and count, record_activity never failing its caller); server tests that each Y2 source records its entry with the right kind, category, actor, summary and subject (a note edit by the user and by an agent, edit coalescing within 60 s, a type change, a chat turn with its tool calls and end tokens, a refusal, a trigger run, an apply, a memory change, a task run, an Outbox send, a device sync round, a login and a failed login), that no message text or secret appears, that /api/activity needs admin and pages, and the activity live event. Targeted tests only (no release yet).

## Worklog
