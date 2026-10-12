---
slug: thevault-prd
kind: spec
created: 2026-10-12
source: MAP.md (D1 to D42), PRD.md
---

# TheVault implementation specification

## Problem

You want one note app that has every feature on your list (D38). Notes are instances of types you define, pages mix rich content with live query views, automation runs on change and on a schedule, and AI agents have the same powers as the UI (PRD). No existing tool does all of this, so TheVault is built from scratch in an empty repository.

## Solution

TheVault is a single-user server written in Rust, with a SolidJS 1.9 web client and a `vault` CLI (D19, D32). The vault is a folder of WCL files, and those files are the source of truth (D2). Every change goes through one write path in the server. That path validates the change, runs `before_*` handlers, writes every affected file atomically, records history, updates the index, and queues `after_*` handlers (D4, D9, D12, D13). Reads are served from a SQLite index and a tantivy full-text index, both rebuilt from the files (D2, D16, D25). wscript runs handlers, scheduled tasks and migrations inside a sandbox the server controls (D6, D9, D10, D30).

The HTTP API is the only way to change the vault. The web client and the CLI are both clients of it (D28, D32).

## User stories

1. As the user, I define a type with fields, relationships and templates in the UI, and every note of that type validates against it.
2. As the user, I add a required field to a type with a default, and every existing note receives the default after I confirm a preview.
3. As the user, I add a task to a project through a two-way reference, and the project's file and the task's file both show the link.
4. As the user, I delete a person referenced by meetings with `on_delete = unlink`, and the meetings lose the reference and their `after_update` handlers run.
5. As the user, I try to delete a project referenced with `on_delete = restrict`, and the delete is refused with the referencing notes listed.
6. As the user, I add a note to a peer relationship, and every other member's list gains it.
7. As the user, I build a page with headings, formatted text, an image, a code block, a terminal session, a Mermaid diagram and an ink drawing, all edited WYSIWYG.
8. As the user, I add a query view block with saved table, kanban and calendar views of the same query and switch between them.
9. As the user, I drag a kanban card to another column, and the note's taxonomy field changes.
10. As the user, I pin notes with locations on a real-world map, and on a map made from my own image.
11. As the user, I place query results on a canvas embedded in a note's template.
12. As the user, I write a `before_update` handler that computes a field, and the field updates on save and stays read-only in the UI.
13. As the user, I write a scheduled task that calls an allowlisted API with a stored secret and updates notes, and I read its run history.
14. As the user, I restore an earlier version of a note, and I recover a deleted note from the trash.
15. As the user, I install the web app as a PWA on a tablet and draw with a stylus.
16. As an AI agent with a `write` token, I list the vault's types, dry-run a batch import, run it, and read back the canonical WCL.
17. As an AI agent, I upsert notes by a key field, so a repeated import changes nothing.
18. As an AI agent with a `write` token, I try to change a type schema, and the API refuses because that needs `admin`.
19. As an AI agent, I write a note that the user changed after I read it, and I get a 409 instead of overwriting their change.
20. As the user, I run `vault init --starter` and get Task, Project, Person and Note types, a Status taxonomy, and a dashboard note.

## Implementation decisions

### Components

The server is split into the components below. The names are working names for crates or modules. The map settles what each component does, not the exact crate boundaries.

| Component | Responsibility | Decisions |
|-----------|----------------|-----------|
| `store` | Vault folder layout, WCL read, canonical WCL write, data-only check, atomic multi-file commits | D2, D4, D20, D21, D22, D23 |
| `schema` | Type, field, relationship, taxonomy and template definitions, note validation, schema-change planning | D3, D5, D6, D7, D14 |
| `writer` | The single write path that every change goes through | D4, D5, D9, D12, D13, D30 |
| `index` | `.vault/index.db` (SQLite) and the tantivy index, incremental updates, full rebuild | D2, D16, D25, D28 |
| `state` | `.vault/state.db`: version history, trash, encrypted secrets, API tokens, task run logs | D10, D11, D13, D25 |
| `query` | Parser, evaluator over the index, builder model, cursor pagination | D8, D27, D33, D42 |
| `script` | wscript host: handlers, tasks, migrations, host functions, resource limits | D6, D9, D10, D30, notes/01 |
| `scheduler` | Cron triggers, retries with backoff, run records, injectable clock | D10 |
| `api` | HTTP API, password session and token auth, scope checks, conflict responses | D11, D12, D21, D32 |
| `cli` | The `vault` binary, an HTTP client of `api` | D25, D28, D32, D37 |
| `web` | SolidJS 1.9 client: editor, templates, views, PWA | D14, D18, D19, D29, D34, D36, D41 |

### Vault folder

The layout is fixed (D22):

| Path | Holds |
|------|-------|
| `notes/<type>/<ULID>.wcl` | One note, with its content blocks inline |
| `types/<type>.wcl` | A type's schema and named templates |
| `taxonomies/*.wcl` | Taxonomies |
| `scripts/*.ws` | wscript handlers and migrations |
| `tasks/*.wcl` | Scheduled task definitions |
| `assets/<ULID>/<filename>` | Uploaded files |
| `.vault/index.db` | Index, rebuilt from the files |
| `.vault/state.db` | History, trash, secrets, tokens, run logs. Not rebuildable |

`store` enforces these rules on every file it writes or loads:

- Vault files contain no expressions and no imports. `store` rejects a submitted file containing either, with a diagnostic naming the offending item (D20).
- `store` loads WCL through a loader that cannot reach outside the vault folder, because WCL's default loader falls through to the real disk (notes/01).
- Every write emits the canonical format. Comments are dropped (D21).
- Dates and datetimes are ISO 8601 strings. URLs are strings. References are note ULIDs. `schema` validates them, not WCL (D23).
- Location and date range are structured values validated the same way (notes/26).
- A content field is a sequence of WCL blocks, one per content block (D34). Text inside a text block uses the inline markup in PRD Appendix A (D41).

The exact WCL vocabulary of each file kind is designed during implementation (OQ-S1). This example shows the intended shape of a note file and is not a contract:

```wcl
title    = "Ship the importer"
status   = "Active/In progress"
due      = "2026-11-01"
project  = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA"
estimate = 3

content {
  heading { level = 2  text = "Plan" }
  paragraph { text = "Import from **Capacities** first, then [[01JB8Z4K2MQ7V3X9T5R6W8Y2ZB]]." }
  terminal {
    command { line = "vault import --dry-run export/"  stdout = "42 notes would be created" }
  }
  query_view {
    query = "from task where project == this && !done sort due asc"
    view "open" { kind = "table" }
    view "board" { kind = "kanban"  group = "status" }
  }
}
```

### Atomic commits

One logical change can touch many files: both sides of a two-way link, every member of a peer relationship, cascaded deletes, and schema changes applied to every note (D4, D5, D6). `store` commits all of them or none of them. A crash part way through must leave no half-written relationship once the server restarts. The mechanism is the implementer's choice (MAP accepted unknowns).

### The write path

Every change from the API goes through `writer` in this order. Handler edits re-enter at step 1 (D30).

1. Parse the submitted note. The API accepts WCL text or JSON, because the CLI reads and writes raw WCL (D32).
2. Reject expressions and imports (D20).
3. Validate against the type schema. Computed fields can only be written by handlers (D3).
4. Check the submitted base version against the current version of every note the caller submitted. If any is stale, fail with 409 (D12).
5. Run `before_create`, `before_update` or `before_delete`. A handler can change the note or reject the save. A rejection fails the request with the handler's message (D9).
6. Work out the side effects: the reverse sides of two-way and peer references (D4) and the `on_delete` rules (D5). `restrict` fails the delete and lists the referencing notes. `unlink` and `cascade` add updates and deletes to the same commit.
7. Commit every affected file atomically (D4).
8. Record a new version in `state.db` for every changed note. The version records who made it: the session, a named token, or a named handler (D13, D30). A delete moves the note to the trash (D13).
9. Update `index.db` and the tantivy index.
10. Queue the `after_*` handlers for every changed note (D9).
11. Return the canonical WCL and the new version of every changed note (D21).

A note's version is bumped whenever any write changes it, including a reverse-side update it did not ask for. A client holding an old copy of the other note then gets a 409 (notes/07). The version token itself is opaque to clients. Its format is chosen during implementation.

Handler chains stop at a fixed depth and fail on a cycle. The depth value is the implementer's choice (D9, MAP accepted unknowns).

### Schema changes

- Adding, renaming or removing a field produces a plan listing every note it changes. The API returns the plan without applying it, and a second call applies it as one atomic commit (D6).
- A new required field must carry a default value, and the plan writes it into every existing note. A new optional field writes nothing (D6).
- A schema change that a simple rule cannot express names a wscript migration in `scripts/`, which runs once over every note of the type within the same commit (D6).
- If a note holds data the schema no longer describes, `schema` keeps the data and flags the note. The API and the UI list flagged notes (D6).

### Taxonomies

- Taxonomies are global and hierarchical. A field declares whether it holds one term or several (D7).
- A kanban view requires a single-term field (D7).
- `under(field, "term")` matches a term and every term below it (D7, D42).

### Query language

`query` implements the grammar in PRD Appendix B (D42):

- Clauses come in a fixed order: `from`, `where`, `search`, `sort`, `group by`, `select`, `limit`.
- Expressions use wscript syntax. A dot follows a single reference.
- The host functions are `today()`, `now()`, `days(n)`, `has()`, `under()`, `any()`, `all()` and `linked_from()`.
- `this` is bound to the note that holds the query. A query with `this` run outside a note fails with a clear error.
- `search` runs against tantivy, and the other clauses run against `index.db` (D16).
- Results are paged with a cursor that the API and the CLI expose. `limit` caps the whole result (D33).
- The visual builder edits a structured model of the query, and `query` converts between text and model. Text to model to text must reproduce the original query (D8). A query the builder cannot show opens as text, so the builder never changes a query it does not understand.

### Scripting

- `script` runs wscript with an instruction limit and a call-depth limit per run, plus a wall-clock timeout and response-size limits that `script` enforces itself. wscript has no memory limit, and its instruction limit does not count time spent in host calls (notes/01).
- wscript VMs are not `Send`, so each worker thread owns one VM, and scripts run on a blocking pool (notes/01).
- Scripts never get wscript's `fs` or `process` modules (notes/01).
- Handlers can call host functions to run queries and to create, update and delete notes. Those calls go through `writer` (D30).
- HTTP host functions are available only in `after_*` handlers and scheduled tasks. Each call is checked against the task's or handler's domain allowlist (D10, D30).
- Secrets are read by name from `state.db`. Scripts cannot list secrets (D10).

### Scheduled tasks

- A task file in `tasks/` names a cron expression, a script, a domain allowlist and the secrets it may read (D10).
- `scheduler` records each run in `state.db` with start time, end time, outcome and log output (D10).
- A failed run retries with backoff, and the UI shows tasks whose latest run failed (D10).
- `scheduler` reads time from a clock interface. Tests swap in a controllable clock (S7).

### API

- One HTTP API serves the web client, the CLI and future native clients (D18, D32).
- Resources cover notes, types, templates, taxonomies, scripts, tasks, assets, queries, search, history, trash, secrets, tokens and task runs. Every UI action maps to an API call (PRD).
- The browser logs in with a password and gets a session cookie. Agents send a bearer token (D11).
- Token scopes are checked per request (D11):
  - `read` allows reads only.
  - `write` allows note and asset changes.
  - `admin` allows everything, including types, templates, taxonomies, scripts, tasks and secrets.
- Notes are readable and writable as WCL text or JSON (D32).
- Batch writes take a list of operations and accept a dry-run flag. A dry run returns exactly what the real run would do (D32). Whether a batch commits all or nothing, or item by item, is the implementer's choice. Either way the response reports a result for every item (MAP accepted unknowns).
- An upsert names a key field. A write matching an existing note's key updates that note, and otherwise creates one (D32).
- Schema inspection returns every type with its fields, kinds, relationships and templates (D32).

### CLI

The `vault` binary is a client of the API and adds no behaviour of its own, except for the local folder commands below (D32).

- Every command prints JSON.
- Commands cover note get, put, delete and history, batch apply with `--dry-run`, upsert, asset upload, query, search, and schema inspection.
- `vault init` creates an empty vault folder. `vault init --starter` adds the starter types, the Status taxonomy and the dashboard note (D37).
- `vault reindex` rebuilds `index.db` and the tantivy index from the files and reports every file that fails validation (D28).
- `vault backup` writes one archive of the vault files and `state.db` (D25).
- An agent skill document that teaches the CLI ships with the binary (D32).

### Web client

- SolidJS 1.9 (D19).
- The editor is TipTap on ProseMirror. TheVault adds an adapter that renders a Solid component inside a ProseMirror node view, and every rich block uses it (D29, notes/03).
- The block types are heading, paragraph, list, quote, image and asset, code, terminal, Mermaid, ink, query view and canvas (PRD).
- Code blocks use CodeMirror 6 (D29).
- Mermaid 12 renders in the browser (D36).
- Ink uses perfect-freehand with pointer events, so stylus, touch and mouse all work. The same component serves ink blocks and ink fields (D18, D29).
- Query views render as table, cards, list, details, kanban, calendar, timeline, map and canvas. Every view pages or virtualises (D33).
- Kanban columns come from a single-term taxonomy field. Dropping a card writes that field through the API (D7).
- The calendar uses FullCalendar's MIT core. The timeline uses vis-timeline (D29).
- The map uses OpenLayers with OpenFreeMap tiles by default, a configurable tile URL, and OpenStreetMap attribution. An image map uses a pixel coordinate system with the same pins (D29).
- The canvas renderer is built for TheVault's own WCL canvas format, and canvas nodes can hold query results (D15).
- The template editor is WYSIWYG. It places fields and blocks on tabs and edits the default, card and detail templates (D14).
- The visual query builder edits the model from `query` (D8).
- A 409 from the API opens a conflict view showing your version beside the stored one (D12).
- The client is a responsive PWA with no offline mode. It targets the last two versions of Chrome, Firefox and Edge, and Safari 17.4 or later (D18, D36).

## Testing decisions

Seven test seams were agreed with you before this spec was written. S1 is the default home for any behaviour.

| Seam | What it observes | Covers |
|------|------------------|--------|
| S1 HTTP API | Black-box requests against a server started on a fresh temporary vault | Types and validation, references and `on_delete`, peer links, schema changes, handlers and their chains, queries, search, history, trash, 409 conflicts, token scopes, upsert, batch and dry run |
| S2 CLI | The real `vault` binary against a running server, asserting JSON output | Schema inspection, batch with `--dry-run` matching the real run, upsert idempotency, asset upload, `init --starter`, `reindex`, `backup` |
| S3 Vault folder | Files on disk after S1 or S2 actions, compared with expected copies | Canonical formatting, both sides of two-way links, layout, rejected expressions and imports, `reindex` rebuilding an identical index, atomicity after a simulated crash |
| S4 Query library | Table-driven tests inside `query` | Every grammar clause and host function against a fixture vault, `this` binding, and text to model to text round trips |
| S5 Browser | Playwright against the running app | WYSIWYG editing, templates with tabs, kanban drag, calendar, timeline, map pins on real and image maps, ink, canvas, the conflict view |
| S6 Benchmark | A generator for a 1M-note vault and timed runs on the reference machine | The p95 targets for query, full-text search, save and note open, and the reindex time (D33, D39) |
| S7 Time and network | A controllable clock flag that only test builds accept, and a local stand-in HTTP server | Cron firing, retry and backoff, the domain allowlist, secret reads, HTTP timeouts and size limits |

The release is done when every feature is in and you sign it off as ready for use (D40). The sign-off includes two checks no test automates:

- Going through every UI action and finding its matching API call and CLI command.
- An agent with only the bundled skill document discovering the types, importing a batch, and fixing its own validation errors.

## Out of scope

- More than one user, sharing, and real-time collaboration.
- Native mobile and desktop clients, which come later through the same API (D18).
- Offline mode (D18).
- Built-in importers (D17).
- JSON Canvas import and export (D26).
- Git integration (D24).
- Watching the vault folder for direct edits (D28).
- Built-in reminders and push notifications (D35).
- wdoc and any other use of WCL beyond data storage (D20).
- The Solid 2.0 migration (D19).
- Format-preserving WCL edits upstream (D21).

## Open questions

- **OQ-S1.** The exact WCL vocabulary for note, type, template, taxonomy, task and canvas files is not settled. The map fixes the layout, the value encodings and that content is structured blocks (D22, D23, D34). The block and field names are not fixed. Settle them before `store` and `schema` are built, because agents, the CLI and S3's expected files all depend on them.
- **OQ-S2.** The map does not choose the Rust HTTP framework, the SQLite crate, or how `state.db` encrypts secrets. These are implementation choices, but secret encryption needs a key source, and the map does not say where that key lives.
- **OQ-S3.** The map does not settle the password store or the login flow beyond "password and session cookie" (D11), including how the first password is set on a new vault.
- **OQ-S4.** The palette for `{colour}` inline text is fixed but not enumerated (D41).
- **OQ-S5.** With up to hundreds of thousands of files in one `notes/<type>/` folder, the folders may need to be split into subfolders (MAP accepted unknowns). The layout contract in D22 does not allow that yet. Decide before S6 runs, because changing it later moves every file.
