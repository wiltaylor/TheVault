---
id: '118'
title: The vault CLI skill teaches authoring types, taxonomies, templates, scripts,
  handlers, tasks and secrets, with stale facts fixed and a skillcheck test
blocked_by: []
status: claimed
verified: unknown
---

# The vault CLI skill teaches authoring types, taxonomies, templates, scripts, handlers, tasks and secrets, with stale facts fixed and a skillcheck test

The user asked (2026-10-18) how well crates/cli/SKILL.md (printed by `vault skill`) sets an AI agent up to work with TheVault, and chose A1, A3 and A4 of the review below (not A2: do not embed docs/vault-format.md or docs/scripts.md in the binary). The skill must therefore carry enough syntax itself for an agent with only the `vault` binary and the skill to author types, taxonomies, templates, scripts, handlers and tasks.

Review findings: F1 no type authoring (type file format, field attributes including labels, accept, of, relation/inverse, hidden, computed; `vault type put`; planned changes with the plan token, renames and migrations); F2 no taxonomy or template authoring (tabs, slots, query views and view kinds); F3 nothing on scripts, handlers, tasks or secrets (wscript basics, host modules per kind of run, run limits, handler events, task files with cron, timeout, allow_domains, secrets, paused; `vault script`, `vault task`, `vault secret`; running a task and reading its run log); F5 stale facts (says two built-in types, there are now dailynote, fleeting, message, call, conversation, person, email, email_thread, event; predates secrets::set, shipped defaults and upgrade steps, person.emails with labels).

A1: add sections, each with one worked example an agent can copy: create a type; add a field to a type that already has notes (plan then apply); create a taxonomy and add terms; write a template with tabs, slots and a query view; write an after_create handler that stamps a field; schedule a task that posts a notification, run it now, and read its run log; set a secret and read it from a task. Take every fact from the code and docs at this commit (docs/vault-format.md, docs/scripts.md, crates/schema, crates/scheduler, `vault <command> --help` via ./dev cargo run -p cli --bin vault). Run every example command against a real test server and fix any that fail; keep the worked examples short.
A3: fix the stale facts in F5 and anything else found untrue at this commit.
A4: a skillcheck test beside the skill, crates/cli/test.py.

Writing standard for SKILL.md (agent-writing skill, applied by the lead): the agent's process is the product; state the positive target behaviour, keep a prohibition only as a hard guardrail paired with its positive; keep the existing frontmatter name; the `description` is the trigger pointer: front-load it and give one trigger per distinct branch (notes, queries, imports, types and schema changes, taxonomies and templates, scripts handlers and tasks, secrets, devices), no synonym pairs, max 1024 chars; preserve every existing instruction (only reword what is stale or unclear, leave passages that already read well untouched so the diff is small); co-locate each concept's definition, rules and caveats under one heading; every step ends on a checkable criterion (for example "vault type get <name> shows the field", "the run's outcome is succeeded"); every meaning in one place; cut lines the model would obey by default; every path, command, flag and count true at this commit. Keep it one file: do not split into topic files. Sprawl is the risk: aim for each new section to be the minimum an agent needs, with closed lists (field kinds, field attributes, handler events, host modules per run kind, view kinds) as compact tables.

Test standard (skillcheck-tests skill, applied by the lead): crates/cli/test.py, canonical shape (shebang `#!/usr/bin/env -S uv run --script`, `# /// script` block with requires-python >=3.11 and dependencies ["pytest-skillcheck"], `from skillcheck import main`, footer `raise SystemExit(main(__file__))`), no conftest.py, no `skill=` argument. A pytest fixture in the file starts a real vault server on port 0 in a fresh temp vault (the host has no Rust toolchain: build with ./dev, and run the server and the `vault` CLI through ./dev; put a small `vault` wrapper script that runs the built CLI through ./dev first on PATH), creates an admin token, and sets VAULT_URL and VAULT_TOKEN in the environment the agent inherits; it never touches the user's live server or $HOME. Three tests, each a lazy, realistic user prompt that does not spell out the procedure, each asserting the vault's end state through the API rather than the transcript: (1) "add a contact type with a list of phone numbers that each have a label" -> a type whose list field has `labels` naming a list of string; (2) with a seeded type that has notes, "add a priority field to my task type" -> the field exists and the existing notes are unchanged and valid; (3) "every new invoice should get today's date stamped in received, and remind me every Monday 9am to review invoices" -> an after_create handler stamps the field on a newly created note (create one through the API and observe it), and a task exists whose manual run succeeds and adds a notification. Each test also asserts `not result.reached_home()`. Run `skillcheck lint crates/cli` (via `uvx --from pytest-skillcheck skillcheck lint crates/cli` or the documented way) and fix its findings. Do not run the live tests (each spends real model calls): leave that to the user, and say so.

Acceptance criteria:
- SKILL.md has the A1 sections with worked examples, and every example command was run against a real server at this commit and succeeded (list them in the worklog).
- The F5 stale facts are fixed; the built-in type list matches crates/writer/src/builtin.rs.
- crates/cli/test.py exists in the canonical shape with the fixture and three tests above, is executable, and `skillcheck lint` passes.
- The CLI test that checks `vault skill` output (crates/cli/tests) still passes; ./dev just check passes.

## Worklog

### 2026-10-18T04:20:46Z — /work

Claimed.
