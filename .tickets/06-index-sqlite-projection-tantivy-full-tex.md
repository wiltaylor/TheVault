---
id: '06'
title: 'index: SQLite projection, tantivy full-text, reindex'
plan: thevault-prd
blocked_by:
- '04'
status: todo
verified: unknown
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
