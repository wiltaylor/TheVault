---
id: '04'
title: 'schema: types, field kinds, validation, taxonomies'
plan: thevault-prd
blocked_by:
- '03'
status: todo
verified: unknown
---

# schema: types, field kinds, validation, taxonomies

Implement the `schema` crate: parse type and taxonomy definitions and validate notes.

Scope:
- Every field kind and flag in `docs/vault-format.md` (D3), required/optional, `computed`.
- Reference fields: target type, single or list, one-way/two-way/peer, `on_delete` (D4, D5). Validation only here; the write-side effects are ticket `refs`.
- Global hierarchical taxonomies, single or multi-valued fields (D7). Kanban-eligibility (single-term) is queryable.
- Value encodings validated per D23 (ISO 8601, URL, ULID), location and date range structures.
- Named templates parsed and validated against the type's fields (D14).

Acceptance criteria:
- A table-driven test covers a valid and an invalid value for every field kind.
- A reference to a missing note, or to a note of the wrong type, fails validation.
- A taxonomy term that does not exist in the taxonomy fails validation; a term deep in the hierarchy passes.
- A template naming a field the type does not have fails validation.

Seam: unit tests in the crate; S1 coverage arrives with ticket `writer`. Decisions D3, D4, D5, D7, D14, D23.

## Worklog
