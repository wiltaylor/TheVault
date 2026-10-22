---
id: '125'
title: lopdf 0.42 (and so pdf-extract) refuses some bank statement PDFs with 'invalid
  file trailer' (PDF-1.3, /Size 11 trailer with ID array, from a Gmail attachment),
  so their text is not searchable
blocked_by: []
status: done
verified: verified
---

# lopdf 0.42 (and so pdf-extract) refuses some bank statement PDFs with 'invalid file trailer' (PDF-1.3, /Size 11 trailer with ID array, from a Gmail attachment), so their text is not searchable

## Worklog

### 2026-10-22T02:09:27Z — wil

Status set to todo.

### 2026-10-22T02:10:12Z — wil

Claimed.

### 2026-10-22T02:18:15Z — wil

Done on ticket/125 (f47e52e). Cause: the statement's xref entries end in a bare LF ('0000000015 00000 n\n', 19 bytes); lopdf 0.42's xref parser wants ' \n', ' \r' or '\r\n', so the table fails and it reports 'invalid file trailer'. The file is also RC4-40 encrypted (V1 R2, empty user password), which lopdf decrypts once loaded. lopdf has no repair load option, so new crates/index/src/pdf_repair.rs rebuilds the xref by scanning 'N G obj' at line starts (skipping stream data, later object wins), copies the last trailer minus /Size /Prev /XRefStm and appends both; extract_with retries both extractors on the rebuilt bytes when both failed, returning Recovered with reason '...; read after rebuilding its xref table'. No new dependency. Test: crates/index/tests/hostile_pdf.rs a_trailer_lopdf_refuses_is_read_after_rebuilding_the_xref_table (crafted PDF-1.3, /Info, /ID array, 19-byte entries). Read-only copy of the live stmt_116989.pdf now extracts 198 words of plain text (uncommitted check). ./dev just check exit 0.

### 2026-10-22T02:18:16Z — wil

Verified set to verified.

### 2026-10-22T02:18:17Z — wil

Status set to done.
