---
id: '124'
title: 'PDF text extraction panics on some real PDFs (pdf-extract 0.12.1: ''second
  arg must be a dict'', ''missing unicode map and encoding''), leaving them unsearchable
  and printing panics on every attempt'
blocked_by: []
status: todo
verified: unknown
---

# PDF text extraction panics on some real PDFs (pdf-extract 0.12.1: 'second arg must be a dict', 'missing unicode map and encoding'), leaving them unsearchable and printing panics on every attempt

Found on the live server, 2026-10-18, after the Gmail sync started importing PDF attachments. The server's pdf-text thread printed: thread 'pdf-text' panicked at pdf-extract-0.12.1/src/lib.rs:1499:68: second arg must be a dict: ObjectType { expected: Dictionary, found: Reference }, and at lib.rs:835:75: missing unicode map and encoding. The panic stays inside the thread and the server keeps running, but the PDF gets no text in the search index. Fix direction: run extraction under catch_unwind (or a subprocess), record the asset as not extractable with the reason so it is not retried on every start, log one line instead of a panic trace, and consider a more tolerant extractor (lopdf text fallback or pdfium). Reproduce with the attachments in the live vault's assets (the server log names no asset; add the asset id to the log line).

## Worklog

### 2026-10-18T05:39:15Z — wil

Status set to todo.

### 2026-10-18T05:39:15Z — wil

User asked to fix it now. Acceptance criteria: (1) a PDF that makes pdf-extract panic is caught (no panic trace printed), logged once as one line naming the asset id and the reason, and recorded so it is not retried on later starts or runs; (2) where a fallback extractor (for example lopdf's own text extraction, already a dependency of pdf-extract) gets text from such a PDF, the text is indexed and searchable; (3) tests with crafted PDFs that trigger both panics seen live (a font whose descendant is a Reference where a dict is expected, and a font with no ToUnicode map or encoding) prove (1) and (2); (4) ./dev just check passes.
