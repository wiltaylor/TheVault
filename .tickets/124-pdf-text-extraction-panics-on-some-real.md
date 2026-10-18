---
id: '124'
title: 'PDF text extraction panics on some real PDFs (pdf-extract 0.12.1: ''second
  arg must be a dict'', ''missing unicode map and encoding''), leaving them unsearchable
  and printing panics on every attempt'
blocked_by: []
status: done
verified: verified
---

# PDF text extraction panics on some real PDFs (pdf-extract 0.12.1: 'second arg must be a dict', 'missing unicode map and encoding'), leaving them unsearchable and printing panics on every attempt

Found on the live server, 2026-10-18, after the Gmail sync started importing PDF attachments. The server's pdf-text thread printed: thread 'pdf-text' panicked at pdf-extract-0.12.1/src/lib.rs:1499:68: second arg must be a dict: ObjectType { expected: Dictionary, found: Reference }, and at lib.rs:835:75: missing unicode map and encoding. The panic stays inside the thread and the server keeps running, but the PDF gets no text in the search index. Fix direction: run extraction under catch_unwind (or a subprocess), record the asset as not extractable with the reason so it is not retried on every start, log one line instead of a panic trace, and consider a more tolerant extractor (lopdf text fallback or pdfium). Reproduce with the attachments in the live vault's assets (the server log names no asset; add the asset id to the log line).

## Worklog

### 2026-10-18T05:39:15Z — wil

Status set to todo.

### 2026-10-18T05:39:15Z — wil

User asked to fix it now. Acceptance criteria: (1) a PDF that makes pdf-extract panic is caught (no panic trace printed), logged once as one line naming the asset id and the reason, and recorded so it is not retried on later starts or runs; (2) where a fallback extractor (for example lopdf's own text extraction, already a dependency of pdf-extract) gets text from such a PDF, the text is indexed and searchable; (3) tests with crafted PDFs that trigger both panics seen live (a font whose descendant is a Reference where a dict is expected, and a font with no ToUnicode map or encoding) prove (1) and (2); (4) ./dev just check passes.

### 2026-10-18T05:40:36Z — /work

Claimed.

### 2026-10-18T05:52:58Z — wil

Verified set to verified.

### 2026-10-18T05:52:58Z — wil

Status set to done.

### 2026-10-18T05:52:58Z — wil

Done on ticket/124 (323f211). The 'second arg must be a dict' panic is not a font descendant as guessed: it is a Separation colour space whose Lab alternate names its dict by reference (pdf-extract lib.rs:1499). The other is a simple font with a ToUnicode map missing a used code and no Encoding (lib.rs:835). Fix: a once-installed panic hook silences panics on 'pdf-text' threads and keeps the message; extract_pdf_text tries pdf-extract, then lopdf extract_text_chunks (same lopdf 0.42 as pdf-extract, no new crate in Cargo.lock); Extracted gained Recovered{text,reason} and Failed(reason); Index::set_asset_text logs one eprintln line per asset naming id and reason, and an unreadable one gets <ULID>.unreadable beside its empty .txt (removed on forget). Live vault copies (32 PDFs): 25 pdf-extract text, 4 recovered by lopdf (2 per panic kind), 2 empty text (scanned), 1 unreadable: 'invalid file trailer' from lopdf, filed as 125. Tests: crates/index/tests/hostile_pdf.rs, crates/writer/tests/pdf_text.rs (child-process test asserting stderr has no panic trace and one line per asset over two starts). ./dev just check exit 0.
