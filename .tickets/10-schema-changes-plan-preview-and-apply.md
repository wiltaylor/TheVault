---
id: '10'
title: 'Schema changes: plan, preview and apply'
plan: thevault-prd
blocked_by:
- '07'
status: done
verified: verified
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

### 2026-10-12T13:06:19Z — /work

Claimed.

### 2026-10-12T13:13:42Z — wil

Verified set to verified.

### 2026-10-12T13:13:42Z — wil

Status set to done.

### 2026-10-12T13:13:42Z — wil

Branch ticket/thevault-prd-10, commit 49abc16. API: POST /api/types {name,wcl}; PUT /api/types/{name} {wcl,renames?,plan?} plans without 'plan' and applies with the token; DELETE /api/types/{name} refuses with 409 type_in_use while notes exist; GET /api/flagged lists notes with their unknown items. The plan token is a sha256 over old and new type text, renames, and every note id+version of the type, so a stale apply gets 409 plan_stale carrying the fresh plan. Validation errors from a type change are 400 {error:invalid, files:[{file,issues}]}. A change that would make previously valid notes invalid (e.g. kind change) is refused listing them; notes already invalid do not block. Admin scope is not enforced: auth is a stub until ticket 08. Schema changes run no handlers; ticket 14 adds migrations to TypeChange. Planning code is schema::FieldChanges (crates/schema/src/change.rs); apply is in crates/writer/src/types.rs, kept out of writer/src/lib.rs to limit conflicts with ticket 09. Note create does not fill defaults (filed as needs-triage).

### 2026-10-12T13:18:19Z — wil

Merged into main at 766f2b0. Merge resolution: ticket 08 had added PUT /api/types/{name} (create, or replace a type with no notes; 409 type_in_use when it has notes). Kept it unchanged and moved plan/apply to POST /api/types/{name}/change; dropped this ticket's POST /api/types since PUT already creates. Type-change errors now use main's 400 invalid_type {failures:[{file,issues}]}. DELETE /api/types/{name} with notes is 409 type_in_use with a notes list (writer Error::TypeHasNotes). just check green after the merge.
