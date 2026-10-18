---
id: '119'
title: Template tabs take a when condition in the query where syntax, evaluated live
  in the web client, so a tab only shows for notes that meet it
blocked_by: []
status: done
verified: verified
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

### 2026-10-18T04:32:43Z — wil

Added by the user: the agent skill (crates/cli/SKILL.md, printed by vault skill) must teach the new tab condition. Ticket 118 adds a template-authoring section to that file; once 118 is merged to main, merge main into this branch and add to that section: the when attribute on a tab, its syntax with one worked example, the fallback rules (active tab hidden, all hidden), and that it needs apps at 0.1.10 or later; run the example against a real server. Acceptance: the skill's template section documents when with a worked example that was run.

### 2026-10-18T04:33:02Z — /work

Claimed.

### 2026-10-18T05:03:40Z — wil

Verified set to verified.

### 2026-10-18T05:03:40Z — wil

Status set to done.

### 2026-10-18T05:03:40Z — wil

Done on ticket/119 (bc5fa14, not merged). The query parser, AST and printer moved to a new crate crates/query-syntax (no deps) so schema can parse a tab's when without a schema<->query cycle; query re-exports everything, and Query::search_all became the free fn query::search_all. Shared cases: crates/query/tests/when_cases.json (58 cases), run by crates/query/tests/when.rs (run_scan with 'id == this && (when)', errors count as false) and web/src/when.test.ts. Web evaluator: web/src/when.ts. A field keeps one slot per template even across conditional tabs; alternate tabs show it with {{field}} text (documented in vault-format.md and the skill). viewTemplate/cardTemplate no longer slice to the first tab; TemplateBody picks the first visible tab for cards. Full Playwright run: 309 passed, 1 failed in google-connect.spec.ts, unrelated, filed as 123. FORMAT_VERSION left at 1 per the ticket's rollout (apps 0.1.9 self-update first).
