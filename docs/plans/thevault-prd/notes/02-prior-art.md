---
id: "02"
kind: research
---

# Local prior art

## Question

Which past projects under `/home/wil/dev` overlap with TheVault, a web note tool with user-defined note types, query-driven views, scripting, an HTTP API with a CLI, and WCL storage? For each: what it is, its stack, its data-model vocabulary, what TheVault can reuse, and what was abandoned and why.

Scanned: lifeos, lifeos-t01, obsidian-oss, llm-wiki, llm-wiki-inbox, ai-kg, muse, reference, plus the full directory listing. `muse` is an empty directory. `reference/` holds only `indidevdan/agentic-prompt-engineering`, a third-party prompt-engineering repo. Neither is relevant. No other directory is note-, vault-, wiki- or KG-related. `wcl` is the storage dependency, not prior art.

## Answer

### lifeos, first plan (`.scratch/lifeos/`, untracked): the closest match, abandoned

- **What:** a complete spec for a build-it-yourself single-user life OS, with typed notes, an embedded query DSL, an agent, and connectors (`lifeos/.scratch/lifeos/map.md:7`, `issues/01-initial-charter.md:15-36`). It produced decisions only, with no code.
- **Stack:** Rust server and agent, Postgres, WCL wdoc note bodies, a SolidJS block editor following the forge design language, PWA only, and `uv` Python connectors (`issues/01-initial-charter.md:19-25`).
- **Vocabulary:** *Note Type* (a schema, authored as a "schema note" by the user or agent), *properties*, *edges* (`@edge(kind, targets)`-decorated properties), core edge kinds `belongs_to` and `mentions`, *Note Block Set*, *query block*, *write gate*, PARA *Project/Area* with an *archived* flag (`issues/04-note-type-and-schema-system.md:22-32`).
- **Decisions worth reading:**
  - The property surface is small and closed: `string`, `bool`, `i64`, `f64`, `date`/`datetime`, `Duration`, symbol-set enums, edges, and `list` of these. It deliberately **excluded structs and maps** ("richer shapes become their own note type"), and schemas are closed (`04:30`). TheVault's dicts contradict this. Decide it consciously.
  - An edge is a typed property holding note IDs. wcl has **no first-class connections and no cross-document `@ref`**, so the app index enforces target existence and type (`04:25`, `map.md:15`).
  - Prose links become derived `mentions` edges. They live only in the index and can never satisfy a typed relation (`04:27`).
  - Schema evolution is an eager, agent-proposed migration. Notes it cannot fill are flagged as exceptions, not blocked, and there is no schema versioning (`04:26`).
  - Note IDs are opaque UUIDv7 and the title is a property, so renames never break links (`04:31`, `06:31`).
  - The query language is a purpose-built minimal DSL compiled to SQL. WCL expressions were rejected as a query syntax (no date type, no descending sort, no count/avg). Raw SQL was rejected because it leaks the index schema (`05:24`). The query surface is type, property and edge predicates with one nesting level, FTS, aggregates, and sort/limit (`05:25`). Only `this` and `today` exist as context (`05:27`).
  - Five render forms: table, list, count, board, chart. Inline mutation is whitelisted. Queries run at view time and are never stored, with coarse SSE invalidation (`05:28-30`). A visual query builder was ruled out because the agent builds queries (`05:31`, `map.md:50`).
  - A query block lowers to a code block that shows its source, so a static wdoc export stays honest (`05:32`).
  - Note bodies are a *documentary subset* of wdoc. wdoc's computational and transclusion blocks are excluded. Unknown custom block kinds are rejected at save (`03:28-29`). The inline grammar is frozen: bold, italic, strike, code, link, autolink, footnote; no inline math or images (`03:31`).
  - The WCL text is canonical and the editor JSON is a projection. Saves use optimistic version checks, and one write gate serves every writer, human or agent (`03:33`).
  - Storage: one standalone WCL document per note, a typed-EAV property index, a single `edges(source, kind, dest, provenance)` table, synchronous same-transaction indexing, append-only full-text revisions, content-addressed assets with revision-aware GC, and hard delete blocked by inbound declared edges (`06:24-37`).
  - Upstream wcl work identified: page-level chart, timeline and tree blocks; a columns fix; quote, divider and todo blocks; and validated `date`/`datetime` types (`03:34`, `04:29`). **Check which of these have landed in wcl.**
  - Left unspecified: templates, automation-rule authoring, the graph view, offline conflicts, and backup (`map.md:31-44`).
- **Why abandoned:** superseded by the container plan. The storage decisions "are all answers to a problem Obsidian already solves" (`lifeos/docs/plans/lifeos-container/notes/04-supersede-prior-plan.md:19-28`). `.scratch/` is untracked and at risk from `git clean` (`MAP.md:31-32`).

### lifeos and lifeos-t01, container plan: active, a different shape

- **What:** one Docker image running `obsidian-headless` sync, herdr, Claude Code and Codex, and a supercronic scheduler. A job is a declaration file that runs a Python script or an agent invocation (`lifeos/docs/plans/lifeos-container/MAP.md:8-20`, D7, D10, D12 at `:49-54`). Status: `building` (`MAP.md:3`). `lifeos-t01` is a worktree with the image scaffold (commit `5f210b1`).
- **Relevant decisions:**
  - Electron was rejected, so the plan gave up Dataview and Templater *results*. An agent reads the query *source*, which it can reason about at least as well (`notes/08-no-electron.md:16-28`).
  - Agents read and write the vault as plain files (D5).
- **Reuse:** the job declaration and scheduler contract (`notes/16-job-contract.md`, `notes/11-scheduler.md`) as a model for TheVault's scheduled tasks.

### llm-wiki: active, an agent-maintained Obsidian vault

- **What:** a personal-admin knowledge base. The agent writes every page under `wiki/` and the user reads it in Obsidian (`llm-wiki/CLAUDE.md:3`). It has five commits, from "Vault created" to "migrate | capacities" (2026-09-04).
- **Stack:** markdown with YAML frontmatter, Obsidian with Dataview, and `tools/wikilint.py`.
- **Vocabulary:** *page types* are one folder each, with a `type:` matching the folder (`CLAUDE.md:25-37`). Ten types: people, organisations, places, things, accounts, health, employment, infra, topics, sources (`.claude/reference/pages.md:5-43`). Fields include `kind:` enums and relationship fields holding `[[links]]` that point from the dependent page to the page it depends on. **Backlinks supply the reverse; a page never hand-lists what points at it** (`pages.md:3`). `dashboards/` contain Dataview queries only (`pages.md:47`, `wiki/dashboards/upcoming.md`).
- **Lesson:** this is a concrete schema set for seeding TheVault's built-in or example types. The upcoming-dates dashboard needs a chain of `choice()` and `default()` calls over six date fields, which suggests first-class date semantics or a "dated obligation" concept. `deadlines.md` is hand-maintained because the query could not replace it (`CLAUDE.md:11,37`).

### llm-wiki-inbox: Capacities and Notion exports, migration source data

- **Capacities:** 28 object-type folders, such as People, Organisation, Accounts, InfrastructureNodes, Software, Queries and DailyNotes. Each object has typed frontmatter (`type: 'Person'`, `collections`, `tags`, `related: []`, typed relation fields such as `organisation: []`), as in `capacities/Codex Astartes /People/*.md`. Queries are themselves objects (`Queries/*.md`).
- **Notion:** a PARA workspace (Projects, Areas, Resources, Archive, Tasks), with each database exported as `.md` plus `_all.csv` (3,533 files).
- **Lesson:** Capacities is the user's previous "object types" tool and the nearest commercial analogue to TheVault. The triage brief notes that the value sits in the frontmatter and the relations, not in the body (`capacities-triage/BRIEF.md:5`). Capacities and Notion importers are realistic day-one needs.

### ai-kg: building, a daemon, CLI and web UI over one HTTP API

- **What:** one background daemon holds a LadybugDB graph of tickets, plans and memory. A CLI and a web UI share **one HTTP API** (`ai-kg/README.md:3-5`, `docs/plans/ai-kg/SPEC.md:25-27`). It has 51 commits, the latest rewiring the planning skills onto it. Status: `building`.
- **Stack:** Python with Starlette, uvicorn, LadybugDB, pygit2 and fastembed (`pyproject.toml:7-15`).
- **Vocabulary:** *record kinds*. *Structural* relations are declared rel tables whose invariants are enforced, such as acyclic `blocked_by`. *Authored links* go in one `Relates` table with a `type` string, a suggested-not-enforced type registry, and `valid_at`/`invalid_at` (`SPEC.md:103-110`).
- **Reuse:**
  - Truth is a git repo of plain record files, and the database is a **rebuildable projection**. Rebuild is the same code path as restore and schema migration (`SPEC.md:127-139`, `:205-210`).
  - Validation runs before the file write (`SPEC.md:134`).
  - The CLI talks over a unix socket with filesystem auth; the browser uses TCP with a minted token (`SPEC.md:79-83`).
  - Read verbs take `--json` for agents (`SPEC.md:181-183`).
  - The web UI writes only what a CLI expresses badly (`SPEC.md:185-187`).

### obsidian-oss: experimental, one commit (2026-09-05)

- **What:** a recovered Obsidian 1.13.7 Electron build, plus an incomplete Tauri port with a **SolidJS** workspace: a file explorer, CodeMirror editor, reading view, command palette and quick switcher (`obsidian-oss/README.md:1-24`, `PORT.md:3-8`).
- **Lesson:** this is UI and command-palette prior art in TheVault's frontend stack. Its own code is recovered and minified, and its licensing is unclear (`RECOVERY.md`), so do not copy code from it.

## Consequences

- **Reuse lifeos plan 1 as the starting decision set.** It already settled most of TheVault's model questions: per-note WCL documents, the write gate, edge-typed properties, derived mentions, an EAV index, a query DSL compiled server-side, view-time queries, opaque IDs, revisions, and asset GC. Cite it as prior art, not as settled decisions, because it was superseded. Preserve `lifeos/.scratch/lifeos/` before it is lost.
- **Decide explicitly where TheVault departs from lifeos plan 1:**
  - **Dicts.** Plan 1 excluded structs and maps.
  - **Two-way and peer relations.** Plan 1 had one-directional edges with backlinks derived by the index.
  - **Query builder.** Plan 1 had an agent-only query builder with no GUI.
  - **View set.** TheVault adds calendar, timeline, map and canvas views; plan 1 had table, list, count, board and chart.
  - **Templates, scripting and schedules.** Plan 1 left these unspecified.
  - **Storage.** Plan 1 was Postgres-only with export, while TheVault stores WCL as truth.
- **Storage truth:** ai-kg's pattern of plain files in git as truth, with the database as a rebuildable projection (one path for rebuild, restore and migrate), fits "WCL as storage format" better than plan 1's Postgres-only store.
- **WCL gaps to check upstream before the PRD commits:** cross-document references, date and datetime types, page-level charts and timelines, and quote, divider and todo blocks. All were missing as of 2026-08.
- **Dynamic views and agents:** an agent reading the query source instead of results is acceptable (lifeos D4). TheVault's API should still expose evaluated query results, because agents using the CLI cannot run views themselves.
- **API and CLI shape:** follow ai-kg. One HTTP API serves the CLI and the UI, read commands take `--json`, and the UI's writes cover what a CLI expresses badly.
- **Seed content and migration:** use llm-wiki's ten page types and the Capacities object types as example schemas and as test fixtures. Treat Capacities and Notion import as candidate features. Date-driven obligations (renewals, expiries) need first-class support, because Dataview needed a hand-kept `deadlines.md` beside it.
- **Avoid** forge code (lifeos found it mid-deletion, `map.md:15`) and obsidian-oss code (recovered, licence-unclear).
