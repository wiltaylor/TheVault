---
id: '22'
title: Type, template and taxonomy editors (WYSIWYG)
plan: thevault-prd
blocked_by:
- '20'
- '16'
status: done
verified: verified
---

# Type, template and taxonomy editors (WYSIWYG)

Build the definition editors (D3, D6, D7, D14).

Scope:
- Type editor: add, rename, remove fields of every kind and flag; reference options (target, cardinality, one-way/two-way/peer, on_delete); defaults required for new required fields; schema-change preview before apply.
- Template editor: WYSIWYG layout of fields and blocks across tabs, for default, card and detail templates.
- Field editors for every field kind used on note pages (date pickers, select, taxonomy picker with hierarchy, reference picker, list, dict, location input, url, computed shown read-only).
- Taxonomy editor with hierarchy.

Acceptance criteria (S5):
- Creating a type with one field of each kind through the UI produces a type file that validates.
- Adding a required field shows the preview listing affected notes and refuses to apply without a default.
- Moving a field to another tab in the template editor changes where it renders on the note page.
- Editing every field kind on a note page saves a value that validates.
- A computed field cannot be edited in the UI.

Seam: S5. Decisions D3, D6, D7, D14.

## Worklog

### 2026-10-12T13:44:00Z — /work

Claimed.

### 2026-10-12T13:58:55Z — wil

Verified set to verified.

### 2026-10-12T13:58:55Z — wil

Branch ticket/thevault-prd-22 @ ca3ce04 (merged ticket/thevault-prd-20 first, which was done but not on main). All five criteria proven by web/e2e/definitions.spec.ts (6 Playwright tests, also 3x --repeat-each green); ./dev just check green; full e2e 20/20. Assumptions: 'every field kind' on note pages = every kind except content (ticket 20's editor), ink (ticket 27) and canvas (canvas ticket), which keep their own editors; the type editor still defines all 18 kinds. The required-field preview lists notes from a query when the plan endpoint refuses (400 invalid_type), because the server does not plan a change without a default. Handlers in a type file are preserved by the type editor but not edited. Template editor adds heading/paragraph blocks only; other blocks already in a template are kept, rendered and movable. No server changes.

### 2026-10-12T13:58:55Z — wil

Status set to done.

### 2026-10-12T14:03:32Z — wil

Merged into main at cb3b44a (ff). Merge of main into the branch conflicted in web/src/api.ts and styles.css; kept both sides. just check green, e2e 29/29 twice.
