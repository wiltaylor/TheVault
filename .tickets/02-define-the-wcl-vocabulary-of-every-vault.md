---
id: '02'
title: Define the WCL vocabulary of every vault file (OQ-S1)
plan: thevault-prd
blocked_by: []
status: todo
verified: unknown
---

# Define the WCL vocabulary of every vault file (OQ-S1)

Settle OQ-S1. Write `docs/vault-format.md` and the WCL schema files that define the exact shape of every vault file. Every later ticket follows this document.

Scope:
- File kinds: note (`notes/<type>/<ULID>.wcl`), type with schema and named templates (`types/<type>.wcl`), taxonomy (`taxonomies/*.wcl`), task (`tasks/*.wcl`), and canvas (a content block and a field value).
- Field kinds from D3 and the brief: string, int, float, bool, date, datetime, date range, url, select, taxonomy, reference (one-way, two-way, peer; single or list; `on_delete`), list, dict, location (geo or image-map), content, asset and ink, with the `computed`, required and optional flags.
- Templates: named templates per type (default, card, detail), with tabs laying out fields and blocks (D14).
- Content blocks as structured WCL blocks (D34): heading, paragraph, list, quote, image/asset, code, terminal (commands with stdout/stderr), mermaid, ink, query_view with saved views, canvas. Inline text uses PRD Appendix A, palette from CLAUDE.md OQ-S4.
- Value encodings from D23: ISO 8601 dates, URL strings, ULID references.
- Data-only rule from D20: no expressions, no imports.
- Ink stroke encoding and the native canvas format (D15): nodes, edges, groups, query nodes, viewport.
- Read `/home/wil/.claude/skills/wcl/SKILL.md` and its references for WCL syntax before writing.

Acceptance criteria:
- `docs/vault-format.md` defines every file kind, field kind, block kind and value encoding above, each with an example.
- `docs/vault-format/` holds one example file per file kind, including a note using every field kind and every block kind, and a canvas.
- Every example passes `wcl check` (install the CLI inside the container with `cargo install --git https://github.com/wiltaylor/wcl.git --rev 0531228286cb19d5b73daf7de23e79790a5e15d4 wcl` or call `wcl_lang` from a small test).
- No example contains an expression or an import.

Spec: SPEC.md Vault folder, OQ-S1. Decisions D3, D4, D5, D7, D14, D15, D20, D22, D23, D34, D41.

## Worklog
