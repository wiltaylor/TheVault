---
id: '22'
title: Type, template and taxonomy editors (WYSIWYG)
plan: thevault-prd
blocked_by:
- '20'
- '16'
status: todo
verified: unknown
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
