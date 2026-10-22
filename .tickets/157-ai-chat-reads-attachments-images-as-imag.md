---
id: '157'
title: 'AI chat reads attachments: images as images, PDFs and text as text'
plan: ai
blocked_by: []
status: done
verified: verified
---

# AI chat reads attachments: images as images, PDFs and text as text

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC.md and /home/wil/dev/TheVault/.scratch/plans/ai/SPEC-2.md (this ticket is SPEC-2 D19).

Add read_asset to the chat tools, image parts in ToolResult with server-side rehydration and downscaling (image crate), the Anthropic and OpenAI wire forms (check OpenAI's live docs for image output in function_call_output; fall back to an appended user message), the 8-images-per-turn limit, fake support for image parts, the thumbnail in the panel's tool step, and docs/ai.md.

Acceptance criteria:
- crates/ai tests: an image part becomes an Anthropic tool_result image block and the chosen OpenAI form; a transcript without parts still deserializes.
- crates/server tests against the fakes: a note with an image asset, read_asset sends base64 image data to the provider and the SSE stream to the client never contains the bytes; a 4000 px image arrives at 1568 px long edge as JPEG; a PDF asset returns its extracted text; an unknown type returns metadata; the 9th image in a turn is refused; replaying done.messages next turn re-sends the image from the vault.
- Web component test: the read_asset step shows the thumbnail.
- ./dev just check passes.

## Worklog

### 2026-10-22T20:51:07Z — wil

Verified set to verified.

### 2026-10-22T20:51:07Z — wil

Status set to done.
