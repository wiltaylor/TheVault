---
id: "26"
kind: research
---

# Do content blocks have a stable identity for three-way merge?

## Question

D20's offline sync three-way merges a note by field and by top-level content block. Do blocks (content-field blocks, columns, list items, ink strokes, canvas nodes) carry a stable id today in the WCL format, the Rust model, or the web editor? If not, how should the merge identify blocks? Can the merge base be fetched by version token?

## Answer

No. Content blocks, columns, list items and ink strokes have no id anywhere: not in the format, not in the parsed model, and not in the editor. Only canvas groups, nodes and edges have ids. These are identifier labels, unique within one canvas. Recommendation: use option 2, a positional diff3 over the canonical text of each top-level block. Keep option 3, the field-level clash, as the fallback when the diff is ambiguous. Do not add block ids. History needs one fix: a version token held by a device can be overwritten before the device syncs.

## Why

- **Format.** A content block "has no label" (`docs/vault-format.md:1054`, `:1062`). The tables for `heading`, `paragraph`, `list`/`item`, `column` and `stroke` list no id item (`docs/vault-format.md:883-950`, `:1064-1085`, `:1339-1346`). A canvas is the exception: `group <id>`, `node <id>`, `edge <id>`, "unique across the groups, nodes and edges of one canvas" (`docs/vault-format.md:1373-1377`). A query view's `place "<ULID>"` is keyed by note id (`docs/vault-format.md:1255`, `:1273`).
- **Validation rejects ids.** `content_item` calls `no_label` on every content block (`crates/schema/src/blocks.rs:320-327`). `items` fails any unknown item with "is not an item of this block" (`crates/schema/src/blocks.rs:214-227`). So an `id` label or item on a block is invalid today. Canvas ids are enforced with `ident_label` plus a duplicate check (`crates/schema/src/blocks.rs:775-795`).
- **Model.** `DocBlock` is `{ kind, labels, body }`, and `Doc` is ordered `fields` and `blocks` (`crates/schema/src/doc.rs:50-64`). Position is the only identity. The writer prints labels as given and keeps no extra state (`crates/writer/src/note.rs:282-310`).
- **Editor.** `web/src/editor/doc.ts` rebuilds every block from ProseMirror JSON on each save (`docToBlock`, `web/src/editor/doc.ts:118-157`). The only attrs it carries are `level`, `checked`, `width` and slot names. Ink, canvas, query views and other blocks go through the editor whole and unchanged as `vaultBlock` attrs (`web/src/editor/doc.ts:74-76`, `:151-152`). Canvas edits therefore keep node ids. New canvas ids come from `freeId` as `prefix`, `prefix_2` and so on (`web/src/canvas/canvas.ts:192-199`). These are deterministic, so two offline devices adding a node can both pick `text_2`. No UniqueID-style extension is installed. The one `id` attr belongs to mentions (`web/src/editor/extensions.ts:91`).
- **History.** The `versions` table stores the whole canonical WCL text of each version (`content TEXT`), keyed by a unique `token` (`crates/state/src/lib.rs:108-118`). `State::version(note_id, token)` fetches one version (`crates/state/src/history.rs:259`). There are two hazards:
  - A browser save within 2 minutes (in a run of at most 10) **updates the head row in place**, with a new token and new content (`crates/state/src/history.rs:54-58`, `:110-115`, `:154-185`). The old token and its text are gone.
  - `prune` deletes old versions by count or age (`crates/state/src/history.rs:350-385`).

## Consequences

**Option 1: add an optional `id` to blocks.** This changes the format, `schema` (relax `no_label` and `items`) and `writer`. The editor needs a UniqueID-style attr on every block node, and that id must survive split, join, wrap into columns, list nesting and undo. Copy and paste must regenerate duplicate ids. Existing notes need a migration, or the server must assign ids on first write. That migration makes one large diff per note in history. Hand-edited WCL, scripts and imports must keep ids too. Highest cost and editor-regression risk.

**Option 2: positional sequence diff.** Hash each top-level block in canonical form. The canonical writer makes this deterministic (D21, `crates/store/src/canonical.rs`). Run diff3 over base, device and server block lists, using LCS on the hashes. Inserts, deletes and edits to different blocks merge cleanly. An edit and a move of the same block, or two edits to the same block, count as a clash. A clash makes a conflict copy, which D20 already provides. Run the diff recursively inside `columns` (per column) and per list item if needed. Canvas merges by node id, which already works, but `freeId` collisions between devices must be treated as a clash or renamed on merge. No format change, all server-side; the cost is more conflict copies after reorders.

**Option 3: field-level only.** Any concurrent change to the same content field is a clash. Trivial and safe, but almost all edits land in `body`, so conflict copies become common.

**Recommendation.** Ship option 2, with option 3 as the fallback when the LCS alignment is ambiguous, for example when blocks repeat. Revisit option 1 only if reorder conflicts prove common in use.

**Base retrievability is required either way.** One of the following is needed:
- Make device writes always add a version, as `record_distinct_version` does (`crates/state/src/history.rs:126-135`). Device writes could also run under a token or device author, which never merges.
- Have the device keep its own base text.

In either case pruning must not delete a base that a registered device still holds. The device could also send the base text with the write. The server keeps whole text per version, so diffing needs no new storage.
