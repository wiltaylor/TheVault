---
id: '16'
title: 'API completeness: definitions, assets, batch, upsert, inspection'
plan: thevault-prd
blocked_by:
- 08
- 09
- '10'
- '12'
status: todo
verified: unknown
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
