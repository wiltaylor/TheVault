---
id: '06'
title: 'index: SQLite projection, tantivy full-text, reindex'
plan: thevault-prd
blocked_by:
- '04'
status: done
verified: verified
---

# index: SQLite projection, tantivy full-text, reindex

Implement the `index` crate: `.vault/index.db` and a tantivy index, both rebuilt from files.

Scope:
- Project notes, field values, references (both directions), taxonomy terms with ancestry, and mentions in inline text into SQLite tables that support the query language.
- Full-text index over field values and content block text (D16).
- Incremental update per commit and full rebuild from the vault folder (D28).
- Rebuild reports every file that fails validation and continues.

Acceptance criteria:
- A full rebuild of a fixture vault followed by a dump of `index.db` equals the dump after building incrementally from the same notes.
- A deliberately invalid file is reported by path with its diagnostic, and every other file is indexed.
- A full-text search for a word in a paragraph block finds the note.
- Deleting `.vault/index.db` and the tantivy directory and rebuilding restores identical query results.

Seam: unit tests, and S3 for the rebuild comparison. Decisions D2, D16, D25, D28.

## Worklog

### 2026-10-12T12:47:31Z — /work

Claimed.

### 2026-10-12T12:54:41Z — wil

Verified set to verified.

### 2026-10-12T12:54:41Z — wil

Status set to done.

### 2026-10-12T12:54:41Z — wil

Branch ticket/thevault-prd-06 at fc672e3, built on ticket/thevault-prd-04 (9a6d544) because 04 was not on main yet: merge 04 first. API: Index::open/rebuild/update(schema, &[Update])/set_schema/search/dump/failures/connection. Tables: notes, field_values(note, field, pos, part, kind, str, num), links(src, via=field|mention, field, pos, dst), terms, term_ancestors(taxonomy, ancestor, path) for under(), failures. num holds int/float/bool and dates/datetimes as Unix seconds. Rebuild drops a type that fails Schema::new and retries, so its dependants fail too, each reported by path. The rebuild comparison runs on files in a temp vault (the S3 seam without S1/S2, which do not exist yet). Writer should call Index::update after each commit and set_schema after schema changes. Full-text scores after incremental deletes can differ slightly from a fresh rebuild (tantivy BM25 stats count deleted docs until merge); the dump and hit sets do not.
