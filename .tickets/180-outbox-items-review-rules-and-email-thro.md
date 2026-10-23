---
id: '180'
title: Outbox items, review rules and email through Gmail with the sent copy
plan: outbox
blocked_by: []
status: done
verified: verified
---

# Outbox items, review rules and email through Gmail with the sent copy

Spec: /home/wil/dev/TheVault/.scratch/plans/outbox/SPEC.md, X1 to X4 and X12's Gmail and format docs. Acceptance: writer tests for every X2 transition by author (user, agent with and without send_without_review, handler, task) including refusals; script or server tests against a fake Gmail (extend testkit's fake Google) that an approved email is sent as RFC 822 with To, Cc, Bcc, Subject, threading headers for a reply, text and html bodies and asset attachments, raw under 4.5 MB and the upload endpoint above, 25 MB refused; sent_ref, sent_at and sent recorded; the copy imported and linked as result once whatever the label settings; no gmail.send scope fails with the reconnect message; can_send in the Google status. Targeted tests only.

## Worklog

### 2026-10-23T09:13:33Z — wil

Verified set to verified.

### 2026-10-23T09:13:33Z — wil

Status set to done.
