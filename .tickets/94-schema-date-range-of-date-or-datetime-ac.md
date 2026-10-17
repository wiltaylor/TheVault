---
id: '94'
title: 'Schema: date_range of = "date_or_datetime", accepted by calendar and timeline
  views and the type editor'
blocked_by: []
status: done
verified: verified
---

# Schema: date_range of = "date_or_datetime", accepted by calendar and timeline views and the type editor

The spec is /home/wil/dev/TheVault/.scratch/plans/google-sync/SPEC.md (gitignored, in the main checkout; read it from that absolute path, since worktrees do not have .scratch). Decision ids D1-D20 there are the google-sync map's, at /home/wil/dev/TheVault/.scratch/plans/google-sync/MAP.md, with reasoning in its notes/; cite them in commits as 'google-sync Dn'. Research on Google's OAuth and API behaviour: /home/wil/dev/TheVault/.scratch/plans/google-sync/notes/01-google-oauth-constraints.md.

Scope: the Schema section of the spec. Acceptance criteria:
- Schema unit tests: a date_or_datetime range accepts two dates or two datetimes, refuses mixed ends and end before start; the query layer sorts and filters such values (a date sorts as the start of its day).
- Web tests: the calendar view places a date-valued range as all-day and a datetime-valued range as timed from the same field; the type editor offers the option.
- docs/vault-format.md documents it.
- ./dev just check passes.

## Worklog

### 2026-10-17T22:58:51Z — /work

Claimed.

### 2026-10-17T23:07:43Z — wil

Verified set to verified.

### 2026-10-17T23:07:43Z — wil

Commit 7e89497 on ticket/94. FieldKind::DateRange is now { of: RangeOf } (Date, Datetime, DateOrDatetime); types.rs diff confined to the DateRange variant, the RangeOf enum and the two date_range lines in the field parser. Calendar placement already branched on value form in narrow.ts; visibleWindow now ORs the local-day and instant windows for an either-form field. The calendar control does not fit an either-form range (server blocks.rs and web controls/model.ts agree). Left alone and filed: 101 (note editor input for either-form ranges), 102 (search page date filters on such fields).

### 2026-10-17T23:07:43Z — wil

Status set to done.
