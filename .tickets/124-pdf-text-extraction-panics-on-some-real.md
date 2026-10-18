---
id: '124'
title: 'PDF text extraction panics on some real PDFs (pdf-extract 0.12.1: ''second
  arg must be a dict'', ''missing unicode map and encoding''), leaving them unsearchable
  and printing panics on every attempt'
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# PDF text extraction panics on some real PDFs (pdf-extract 0.12.1: 'second arg must be a dict', 'missing unicode map and encoding'), leaving them unsearchable and printing panics on every attempt

Found on the live server, 2026-10-18, after the Gmail sync started importing PDF attachments. The server's pdf-text thread printed: thread 'pdf-text' panicked at pdf-extract-0.12.1/src/lib.rs:1499:68: second arg must be a dict: ObjectType { expected: Dictionary, found: Reference }, and at lib.rs:835:75: missing unicode map and encoding. The panic stays inside the thread and the server keeps running, but the PDF gets no text in the search index. Fix direction: run extraction under catch_unwind (or a subprocess), record the asset as not extractable with the reason so it is not retried on every start, log one line instead of a panic trace, and consider a more tolerant extractor (lopdf text fallback or pdfium). Reproduce with the attachments in the live vault's assets (the server log names no asset; add the asset id to the log line).

## Worklog
