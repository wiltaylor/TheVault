---
id: '39'
title: 'web: there is no UI to create a note'
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# web: there is no UI to create a note

Found by the ticket 31 parity audit (D40). No page, button or route in web/src creates a note: api.createNote (web/src/api.ts:297, POST /api/notes) has no caller and App.tsx has no /notes/new route. The API (POST /api/notes) and CLI (vault put --type) both create notes. PRD outcome 1 and every type list assume you can add a note of a type from the UI. See docs/parity.md.

## Worklog

### 2026-10-12T15:56:13Z — wil

Triage (orchestrator): Fix: add note creation in the web UI: a New note action on each type list and a /notes/new?type= route rendering the type's default template as an editable form; server applies defaults (ticket 33). Acceptance (S5): creating a note of a starter type from the UI writes a valid file and opens it.

### 2026-10-12T15:56:13Z — wil

Status set to todo.

### 2026-10-12T15:59:59Z — /work

Claimed.

### 2026-10-12T17:07:31Z — wil

Verified set to verified.

### 2026-10-12T17:07:32Z — wil

Commit 772c089 on ticket/thevault-prd-39. TypeList has a 'New <type>' link to /notes/new?type=<type>; App routes it to NewNote (NoteView.tsx), which renders NoteView with newType: a draft note open for editing, template laid out as usual, fields showing their defaults (not sent unless edited; the server fills them per #33). Save calls api.createNote and navigates to /notes/<id>; Cancel goes back to the type list; unknown type shows 'There is no type X.'. e2e: web/e2e/create-note.spec.ts (4 tests) - the e2e vault now also holds the starter note type, copied from crates/cli/starter/types/note.wcl by global-setup. Evidence: './dev pnpm -C web e2e create-note --workers=1' 4 passed; in a full run '--workers=2' the 4 passed while 33 other specs timed out at 30 s. Unmodified main (b6d940c, my diff stashed) showed the same timeouts in the same specs (admin, canvas, date-views) while t36's 1M-note bench held load avg ~9-13; see #46. './dev just check' passed (check=0). docs/parity.md row for Create a note updated.

### 2026-10-12T17:07:32Z — wil

Status set to done.

### 2026-10-12T17:20:38Z — wil

Merged into main as 704c892 (fast-forward; main had moved by ticket 38, merged main into the branch first and ./dev just check passed again).
