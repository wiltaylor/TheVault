---
id: '21'
title: Image, code, terminal and Mermaid blocks
plan: thevault-prd
blocked_by:
- '20'
- '16'
status: done
verified: verified
---

# Image, code, terminal and Mermaid blocks

Add the media and technical blocks.

Scope:
- Image and asset blocks with upload through the assets API.
- Code block on CodeMirror 6 with a language selector and syntax highlighting.
- Terminal block: a list of commands each with stdout and stderr, edited WYSIWYG, rendered as a terminal session.
- Mermaid 12 block with source editing and live render; render errors shown inline.

Acceptance criteria (S5):
- Uploading an image shows it and stores it under `assets/`.
- A code block keeps literal text exactly (tabs, trailing spaces) through save and reload.
- A terminal block with two commands, stdout and stderr saves and reloads identically and renders stderr distinct from stdout.
- A valid Mermaid diagram renders an SVG; an invalid one shows the error and keeps the source.

Seam: S5. Decisions D29, D34, D36.

## Worklog

### 2026-10-12T13:43:58Z — /work

Claimed.

### 2026-10-12T13:52:50Z — wil

Verified set to verified.

### 2026-10-12T13:52:50Z — wil

Status set to done.

### 2026-10-12T13:52:50Z — wil

Branch ticket/thevault-prd-21 at 506c895. Built on an unmerged ticket/thevault-prd-20 (merged into this branch as df1ef1f, conflicts in crates/api kept main's side). Blocks stay as vaultBlock nodes; BlockHolder now passes onChange so web/src/blocks/* edit them in place. All four criteria are proven by web/e2e/media.spec.ts (S5); global-setup exports E2E_VAULT for the assets/ file check. Left alone: paste/drop of images into the editor (only toolbar upload exists); the Vite build warns about a >500 kB main chunk (warned before this ticket too; Mermaid itself is lazy-loaded).

### 2026-10-12T13:54:12Z — wil

Merged into main as abedb3c (fast-forward). Resolved a crates/api/src/lib.rs conflict with main's side; just check and 19/19 e2e passed after the merge.
