---
id: '103'
title: 'select fields cannot label their options, so the event type''s calendar select
  holds Google calendar ids with no names (google-sync spec: ''options are the synced
  calendars'' ids, labelled by name''); ticket 95 shipped options = ["primary"] unlabelled'
blocked_by: []
status: done
verified: verified
---

# select fields cannot label their options, so the event type's calendar select holds Google calendar ids with no names (google-sync spec: 'options are the synced calendars' ids, labelled by name'); ticket 95 shipped options = ["primary"] unlabelled

## Worklog

### 2026-10-17T23:51:19Z — wil

Resolved for the event type by ticket 98 (lead's decision): event.calendar changed from a select to a string holding the Google calendar id, since a script cannot change a type to fill options; the google_sync note's calendars list holds the ids and default_calendar the default. The general question of labelled select options stays open.

### 2026-10-22T02:09:27Z — wil

Status set to todo.

### 2026-10-22T02:10:01Z — wil

Claimed.

### 2026-10-22T02:47:36Z — wil

Branch ticket/103, commit b32676c (merged with main as 9dfe620). Select fields take option_labels = ["Personal", ...], index-aligned with options (an empty label shows the value); a separate attribute because v0.1.12's reader checks options with Form::StrList and would refuse records, while it ignores option_labels with a warning. Web shows labels in fieldText, Choice/Picker, control dropdown/radio/chips and search filters; type editor takes 'value = Label' lines. Event type: calendar stays a string (a script cannot change a type to fill select options) plus computed calendar_name filled by google_calendar.ws from calendarList (summaryOverride, else summary; primary aliased); new script hash appended to SHIPPED_FILES. Found and fixed: restore from trash refused notes holding computed values. Tests: schema validation option-label tests, server google_calendar/google_types/schema_changes, web options.test.ts, defs.test.ts, e2e type-editor.spec.ts. ./dev just check passes.

### 2026-10-22T02:47:36Z — wil

Verified set to verified.

### 2026-10-22T02:47:36Z — wil

Status set to done.
