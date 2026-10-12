---
id: '04'
title: 'schema: types, field kinds, validation, taxonomies'
plan: thevault-prd
blocked_by:
- '03'
status: done
verified: verified
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

### 2026-10-12T12:37:50Z — /work

Claimed.

### 2026-10-12T12:47:06Z — wil

Verified set to verified.

### 2026-10-12T12:47:07Z — wil

Status set to done.

### 2026-10-12T12:47:07Z — wil

Committed 9a6d544 on ticket/thevault-prd-04. Criteria proven by crates/schema/tests/validation.rs: every_field_kind_accepts_a_valid_value_and_rejects_an_invalid_one (table of 27 rows, asserts a row exists for each of the 18 FieldKind::NAMES), a_reference_to_a_missing_note_fails, a_reference_to_a_note_of_the_wrong_type_fails, a_term_must_exist_in_the_taxonomy, a_deep_term_passes_in_a_deeper_hierarchy, a_template_naming_a_field_the_type_lacks_fails. Mutation-checked: disabling the missing-note, term-existence and slot checks turns those tests red. Assumptions: schema does the semantic layer on the store's AST and is strict about unknown items in definition files, so it does not also run wcl_lang structural validation; a required multiple field must hold at least one value; references inside content blocks (image assets, canvas note nodes, mentions) are checked for ULID form only, not existence; query_view view items (group, date, location) are not resolved against the queried type, that belongs with query. Schema-change planning (D6) is not in this ticket.
