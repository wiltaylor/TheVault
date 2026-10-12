---
id: '16'
title: 'API completeness: definitions, assets, batch, upsert, inspection'
plan: thevault-prd
blocked_by:
- 08
- 09
- '10'
- '12'
status: done
verified: verified
---

# API completeness: definitions, assets, batch, upsert, inspection

Fill the API to cover every UI action.

Scope:
- Endpoints for templates, taxonomies (hierarchy edits), scripts, assets (upload, download, delete), canvas content.
- Batch writes with dry-run whose output equals the real run's result; per-item results (D32, MAP accepted unknown on atomicity: choose and document).
- Upsert by a key field (D32).
- Schema inspection: every type with fields, kinds, relationships and templates (D32).
- An OpenAPI document (or equivalent machine-readable listing) of every endpoint, served by the server.

Acceptance criteria (S1):
- Uploading an asset stores it at `assets/<ULID>/<filename>` and it downloads byte-identical.
- A batch dry run reports the same per-item outcome as the real run that follows, and changes no file.
- Upserting the same batch twice creates notes once and changes nothing the second time.
- Schema inspection lists a type created through the API with all its fields.
- The endpoint listing covers every route the server registers (test compares them).

Seam: S1. Decisions D17, D32.

## Worklog

### 2026-10-12T13:15:03Z — /work

Claimed.

### 2026-10-12T13:29:40Z — wil

Verified set to verified.

### 2026-10-12T13:29:40Z — wil

Status set to done.

### 2026-10-12T13:29:40Z — wil

Branch ticket/thevault-prd-16 @ e21a727. Builds on ticket/thevault-prd-09 (merged into this branch, clean); ticket/thevault-prd-10 is NOT merged here because it conflicts with main (it predates 08's PUT /api/types and Error::TypeInUse(String)). Whoever merges 10 after 16 must add 10's routes (POST /api/types, DELETE /api/types/{name}, GET /api/flagged, and its PUT contract) to crates/api/src/openapi.rs ENDPOINTS and register them through the Routes builder in crates/api/src/lib.rs, or api's openapi unit test and server/tests/completeness.rs fail. Batch atomicity choice: all-or-nothing, one commit; dry run = same path stopped before commit. Two items may not target the same note or upsert key. Upsert merges submitted fields/blocks into the matched note; base_version optional for upsert. Scripts are stored without wscript syntax checking (script crate is still a stub). Taxonomy renames do not rewrite type-field defaults that name a renamed term; such a rename is refused as invalid_type until the type is changed. Tasks endpoints left out (not in this ticket's scope).

### 2026-10-12T13:31:28Z — wil

Merged into main at 4d8339b (fast-forward) after merging main (with ticket 10) into the branch; 10's routes were added to the Routes builder and the openapi ENDPOINTS table. ./dev just check passed on the merged tree.
