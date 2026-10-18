---
id: '119'
title: Template tabs take a when condition in the query where syntax, evaluated live
  in the web client, so a tab only shows for notes that meet it
blocked_by: []
status: todo
verified: unknown
---

# Template tabs take a when condition in the query where syntax, evaluated live in the web client, so a tab only shows for notes that meet it

The user asked (2026-10-18) for conditions on templates so a tab only appears when a condition on the note is met.

Decisions (lead):
1. Syntax: a template `tab` block takes an optional `when = "<expression>"`, written in the query language's `where` expression syntax (PRD Appendix B, docs/plans/thevault-prd/PRD.md; implementation in crates/query), evaluated against the note shown: field names, `==`, `!=`, `<`, `<=`, `>`, `>=`, `&&`, `||`, `!`, parentheses, literals, and the functions the query language offers for a single note (at least has(), under(), and whatever else the docs list that makes sense for one note; a function that needs other notes, if any, is refused in `when`). `this`-relative forms behave as in query views where they apply.
2. Validation: the schema parser checks the expression when the type is saved; a parse error or an unknown field is an error at `template <name>.tab "<label>".when`. Document in docs/vault-format.md under templates, with an example.
3. Evaluation: in the web client, live, as the user edits fields, so a tab appears or disappears without a save. Add a TypeScript evaluator for the `where` subset above (web/src/..., a small module with no UI). To keep Rust and TS semantics the same, add one shared JSON file of cases ({ note fields, expression, expected bool }) that a Rust test (crates/query) and a web test both run. Missing fields behave as the query language treats missing values.
4. UI behaviour: hidden tabs are not shown in the tab bar; if the active tab becomes hidden, switch to the first visible tab; if every tab is hidden, show the first tab, so a note never renders empty. Cards use the `card` template, which may also have a `when` per tab with the same rules. The type editor's template editor shows and edits `when` on a tab, with the parse error shown inline.
5. Older apps: ticket 100 made unknown type and field attributes warnings, but unknown items in templates, slots and handlers are still errors. Extend the same tolerance to attributes inside template blocks (tab, slot, and fixed blocks): an unknown attribute is a warning and ignored. Note in docs/android-release.md that a vault using `when` needs apps at 0.1.10 or later, and that apps from 0.1.9 update themselves before syncing.

Acceptance criteria:
- Schema tests: a valid `when` parses; a syntax error and an unknown field are errors at the tab's path; an unknown attribute on a tab or slot is a warning and the template still loads.
- The shared case file passes in both the Rust test and the web test (at least 25 cases covering each operator, has, under, missing fields, dates and numbers).
- Web component tests: a tab with a false `when` is absent, appears when the field changes without a save, the active tab falls back when hidden, and all-hidden shows the first tab.
- A Playwright check on a note whose type has a conditional tab: editing the field shows and hides the tab; save a screenshot to /home/wil/dev/TheVault/.scratch/shots/ticket-when-tab.png.
- docs updated; ./dev just check passes.

## Worklog
