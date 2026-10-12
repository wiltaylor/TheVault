---
id: '40'
title: 'web: types, taxonomies and assets cannot be deleted from the UI'
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# web: types, taxonomies and assets cannot be deleted from the UI

Found by the ticket 31 parity audit (D40). DELETE /api/types/{name}, DELETE /api/taxonomies/{name} and DELETE /api/assets/{id} exist and have CLI commands (vault type delete, vault taxonomy delete, vault asset delete), but TypeEditor.tsx, TaxonomyEditor.tsx and no asset page offer a delete. Scripts, tasks, templates, secrets and tokens all have one. See docs/parity.md.

## Worklog

### 2026-10-12T15:56:13Z — wil

Triage (orchestrator): Decisions: deleting a type with notes is refused with the note count; deleting a taxonomy term or taxonomy in use is refused listing the notes that use it; deleting an asset referenced by notes asks for confirmation listing them, then removes the references with unlink semantics. Add the UI and any missing API. Acceptance (S1+S5): each refusal and each successful delete is tested.

### 2026-10-12T15:56:13Z — wil

Status set to todo.

### 2026-10-12T17:11:26Z — /work

Claimed.

### 2026-10-12T18:37:42Z — wil

Verified set to verified.

### 2026-10-12T18:37:42Z — wil

Status set to done.

### 2026-10-12T18:37:42Z — wil

Done on ticket/thevault-prd-40 (204c1d2). API: type_in_use gains count; taxonomy_in_use now lists fields plus notes holding its terms; asset_in_use/taxonomy_in_use/term_in_use responses carry notes:[{id,type,title}] (added in api::error::in_use from the by paths); DELETE /api/assets/{id}?unlink=true and vault asset delete --unlink unlink the asset from note fields, image/asset blocks and canvas asset nodes (edges to removed nodes go too) in the same commit. Assumption: an asset a type file names (location image map) stays refused even with unlink, since a type is not a note. UI: Delete type (TypeEditor), Delete taxonomy (TaxonomyEditor), new /assets page with inline confirmation panel. Tests: S1 completeness.rs (3 new), S2 cli.rs (1 new), S5 e2e/deletes.spec.ts (3 new), vitest api.test.ts. Note: e2e and timing-based Rust tests (handlers depth limit, script host create timeout) failed while other workers ran 1M-note benchmarks — the same e2e failures reproduced with main's Rust code; all passed once load dropped.

### 2026-10-12T18:42:35Z — wil

Merged into main at 007c75d (fast-forward after merging main into the branch; ./dev just check exit 0; deletes, create-note, definitions e2e specs pass).
