---
id: '10'
title: 'Schema changes: plan, preview and apply'
plan: thevault-prd
blocked_by:
- '07'
status: todo
verified: unknown
---

# Schema changes: plan, preview and apply

Implement simple schema changes (D6). wscript migrations come in ticket `handlers`.

Scope:
- Type create, update and delete endpoints (admin).
- A schema change returns a plan listing every note it changes, without applying; a second call applies the plan atomically (D6).
- Adding a required field requires a default; existing notes receive it. Adding an optional field writes nothing (D6).
- Rename moves values. Remove drops the field from the schema; notes keep the data and are flagged (D6).
- Flagged notes are listed by an endpoint.

Acceptance criteria (S1 + S3):
- Adding a required field without a default is rejected.
- Adding a required field with a default writes it into every existing note's file after apply, and into none after plan only.
- Adding an optional field leaves every note file byte-identical.
- Renaming a field moves the value in every note.
- Removing a field keeps the data in files and lists the notes as flagged.
- A plan applied after a note changed since planning fails rather than overwriting.

Seam: S1 and S3. Decisions D6.

## Worklog
