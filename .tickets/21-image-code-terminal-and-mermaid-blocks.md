---
id: '21'
title: Image, code, terminal and Mermaid blocks
plan: thevault-prd
blocked_by:
- '20'
- '16'
status: todo
verified: unknown
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
