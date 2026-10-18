---
id: '126'
title: Toggle blocks with a heading or text summary that expand and collapse nested
  blocks, and admonition blocks whose kind sets colour and icon with presets
blocked_by: []
status: todo
verified: unknown
---

# Toggle blocks with a heading or text summary that expand and collapse nested blocks, and admonition blocks whose kind sets colour and icon with presets

The user asked (2026-10-18) for toggle blocks (a summary line, plain text or a heading, always shown, that expands or collapses the blocks nested in it) and an admonition (callout) block whose kind gives it a colour and an icon, with presets such as warning, information and danger.

Decisions (lead):

Toggle, content block kind `toggle`:
- `text`: inline markup, the summary line, always shown. `level`: 0 (plain text, the default) or 1, 2, 3 (shown as that heading level; a toggle heading counts as a heading for any outline or heading list the app builds).
- Nested `blocks`: any content blocks, including toggles and admonitions, as `columns` nests blocks today.
- Open or closed is not stored in the note (so a click is not an edit, a version, or a sync change). Each device remembers it per note and block (local storage in the web client), keyed by a stable block id if blocks have one, else by position. New toggles start closed. A search hit or a link that targets a block inside a toggle opens it.
- Inline markup, mentions and placeholders in `text` behave as in a paragraph (indexed for search, links counted for backlinks); nested blocks are indexed as usual.

Admonition, content block kind `admonition`:
- `kind`: one of note, info, tip, success, warning, danger, question, quote. Each preset maps to a colour from the inline palette (OQ-S4: gray, brown, orange, yellow, green, blue, purple, pink, red) and an icon from the web client's icon set (web/src/components/Icon.tsx; add icons if missing): note gray/pencil, info blue/info, tip green/lightbulb, success green/check-circle, warning orange/alert-triangle, danger red/octagon-alert (or the set's closest), question purple/help-circle, quote brown/quote.
- Optional `colour` (palette name) and `icon` (icon name) override the kind's.
- Optional `title`: inline markup; when absent the kind's label is shown ("Warning").
- Nested `blocks`: any content blocks.
- Optional `collapsible` (bool, default false): when true, the admonition toggles like a toggle block, with the same per-device open state, starting open.

Both:
- Schema: add both kinds to the closed set of content blocks in crates/schema (validation of every attribute, refusal of an unknown kind, colour or icon name with a clear message), the canonical WCL writer, the JSON form for the notes API and scripts (`{ kind, fields, blocks }` as vault::append_blocks takes), the index (text and links), and any export or rendering path that walks blocks (find them: Render.tsx, markdown or plain-text export if present, the card/snippet text).
- Editor (web/src/blocks and the block editor): slash commands `/toggle` and `/callout` (with the kind picker), a toggle shortcut of `> ` followed by a space at the start of an empty paragraph only if `> ` is not already the quote shortcut (if it is, use `>> ` for toggle and say so in the docs), Tab and Shift+Tab to move a block into or out of the toggle or admonition above it, Enter at the end of a toggle summary creates the first nested block when open, Backspace in an empty summary turns the toggle back into a paragraph. A block menu changes an admonition's kind, colour and icon.
- Docs: docs/vault-format.md (content blocks section, with WCL examples), docs/scripts.md (JSON form), and the agent skill crates/cli/skill/thevault/SKILL.md (the content blocks part of writing a note: one example each; its sh blocks are run by a test, so any command added must succeed).
- Compatibility: a note using either kind is rejected by apps before this release, so docs/android-release.md notes that vaults using them need apps at this release or later; apps from 0.1.9 update themselves before syncing.

Acceptance criteria:
- Schema tests: each kind round-trips through WCL and JSON; unknown kind, colour or icon is refused with a message; nested blocks validate.
- Index tests: text in a toggle summary, an admonition title and nested blocks is searchable; a mention inside a toggle counts as a backlink.
- Web component tests: a toggle starts closed, opens and closes, remembers its state across a reload on the same device without saving the note, and opens for a search hit inside it; an admonition renders each preset's colour and icon and an override; collapsible admonition toggles; slash commands insert both; Tab and Shift+Tab nest and un-nest.
- A Playwright check creating a toggle with a heading summary and nested list, and a warning admonition, then reloading; save screenshots to /home/wil/dev/TheVault/.scratch/shots/ticket-toggle-*.png.
- Docs and skill updated; ./dev just check passes.

## Worklog
