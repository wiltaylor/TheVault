---
id: '02'
title: Define the WCL vocabulary of every vault file (OQ-S1)
plan: thevault-prd
blocked_by: []
status: done
verified: verified
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

### 2026-10-12T12:09:42Z — /work

Claimed.

### 2026-10-12T12:23:52Z — wil

Verified set to verified.

### 2026-10-12T12:23:52Z — wil

Status set to done.

### 2026-10-12T12:23:52Z — wil

Branch ticket/thevault-prd-02, commit 2fc4af9. Verify with: ./dev cargo install --git https://github.com/wiltaylor/wcl.git --rev 0531228286cb19d5b73daf7de23e79790a5e15d4 --root $PWD/.scratch/wcl wcl; ./dev env WCL=$PWD/.scratch/wcl/bin/wcl docs/vault-format/check.sh (7 wcl-check ok, 7 data-only ok, both invalid fixtures rejected, exit 0). Upstream WCL quirks at the pinned rev, documented in docs/vault-format.md 'WCL behaviour this format works around': (1) 'wcl check -' on stdin skips schema validation and prints OK for anything that parses -- my first green run was false, caught by negative probes; (2) non-optional fields are not enforced unless listed in @block required_fields, and @document has no equivalent, so required top-level note fields are schema-crate work; (3) @schemaless on a top-level @document field is ignored, hence dict fields are labelled blocks; (4) record keys must be identifiers; (5) record members are not checked (DateRange with only start passes); (6) extends does not inherit @children. Not wired into 'just check' because ticket 01 owns the justfile; a later ticket may add check.sh to the gate. Handler registration syntax (ticket 14) is the 'handler "<event>"' block in type files. data-only.mjs is a reference checker only; the store crate (ticket 03) owns the real one.
