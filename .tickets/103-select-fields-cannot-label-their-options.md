---
id: '103'
title: 'select fields cannot label their options, so the event type''s calendar select
  holds Google calendar ids with no names (google-sync spec: ''options are the synced
  calendars'' ids, labelled by name''); ticket 95 shipped options = ["primary"] unlabelled'
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# select fields cannot label their options, so the event type's calendar select holds Google calendar ids with no names (google-sync spec: 'options are the synced calendars' ids, labelled by name'); ticket 95 shipped options = ["primary"] unlabelled

## Worklog

### 2026-10-17T23:51:19Z — wil

Resolved for the event type by ticket 98 (lead's decision): event.calendar changed from a select to a string holding the Google calendar id, since a script cannot change a type to fill options; the google_sync note's calendars list holds the ids and default_calendar the default. The general question of labelled select options stays open.
