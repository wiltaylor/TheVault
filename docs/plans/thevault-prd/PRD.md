---
slug: thevault-prd
kind: prd
created: 2026-10-12
source: MAP.md (D1 to D42)
---

# TheVault product requirements

## Problem

Note tools make you choose. Free-form editors such as Notion and Obsidian give you rich pages but no enforced structure, so a "project" note in one place has different fields from a "project" note somewhere else, and queries over them return wrong results. Structured tools such as Capacities give you typed objects but no real automation, no API an AI agent can use, and a storage format you cannot read or version yourself.

TheVault is a single-user, self-hosted note system where every note is an instance of a type you define. The type sets the fields, how the note renders, and what happens when the note changes. The vault is stored as plain WCL files. An HTTP API and a CLI expose everything the UI can do, so AI agents can read, write, import and maintain the vault alongside you.

## Who this is for

One person who runs their own server and wants three things from their notes:

- Structure they define, with types, relationships and taxonomies that the system enforces.
- Rich pages built from content blocks and live query views, edited WYSIWYG.
- AI agents that work in the vault through a CLI with the same powers as the UI.

There is no second user. Sharing, permissions between people, and real-time collaboration are out of scope.

## Why now

No existing tool has every feature on this list, and stitching several tools together loses the typed structure that makes queries trustworthy. TheVault exists to be the one note app with all of them. AI agent access is one of those features, not the reason for the product (D38).

## Desired outcomes

Each outcome is something you can observe in the running product.

1. You define a type in the UI, with fields, relationships, taxonomies and tabbed templates, and every note of that type renders and validates against it.
2. You build a page that mixes text, images, code, terminal sessions, diagrams, ink and live query views, and edit all of it WYSIWYG.
3. Dragging a card between kanban columns changes the note's taxonomy value, and the change is visible in every other view of that note.
4. Linking two notes through a two-way or peer relationship updates both notes in one step, and either note's WCL file shows the link.
5. A note save runs its type's event handlers, which can compute fields, reject invalid saves, and update related notes.
6. A scheduled task pulls data from an external API on a cron schedule and updates notes, and its run history and failures show in the UI.
7. An AI agent with a scoped token discovers the vault's types through the CLI, imports a batch of notes with `--dry-run` first, and reads back the canonical WCL.
8. You restore an earlier version of a note, or recover a deleted note from the trash.
9. A vault of 1M notes stays responsive to the targets in the success criteria.

## Requirements

### Types and fields

- You define note types with a schema of typed fields. The field kinds are: string, int, float, bool, date, datetime, date range, url, select (an inline list of allowed values), taxonomy, reference, list, dict, location, content, asset and ink. Sources: brief, D3.
- Each field is required or optional.
- A field can carry a `computed` flag. A computed field is read-only in the UI, and only event handlers write it (D3).
- A reference field names the type it points to. It holds a single reference or a list.
- A reference field can be one-way, two-way or peer:
  - A **two-way** relationship pairs a field on one type with a field on the other. A single reference gives a one-to-one or one-to-many link. A list gives many-to-many.
  - A **peer** relationship is a list of references to the same field on the same type, such as "related projects" on Project. Adding B to A's list adds A to B's list.
  - Both notes in a two-way or peer relationship store the link in their own file. TheVault updates both sides as one atomic change (D4).
- Each reference field declares what happens when the target is deleted: `unlink` (the default), `restrict` (the delete is refused) or `cascade` (the referencing note is deleted too). Unlink counts as an update to the referencing note, and cascade counts as a delete, so their event handlers run (D5).
- A location is either a latitude and longitude, or a point on a custom image map (D3).
- An ink field holds a drawing. The template shows a surface where you can draw on it (brief).

### Taxonomies

- Taxonomies are global and hierarchical, so one Status or Topic taxonomy can serve many types (D7).
- Each taxonomy field holds a single value or several (D7).
- A query for a term matches that term and every term below it (D7).

### Schema changes

- Adding, renaming or removing a field applies to every existing note at once, after a preview of what will change (D6).
- A new required field must have a default value, and existing notes receive it. A new optional field leaves existing notes without a value (D6).
- Changes that a simple rule cannot express run a migration script written in wscript (D6).
- If a note holds data the new schema no longer describes, TheVault keeps that data and flags the note rather than dropping the data (D6).

### Templates

- Each type has several named templates. One is the default, and you can add card and detail variants that query views use (D14).
- A template arranges fields and content blocks across tabs, and you choose what each tab shows (brief).
- You edit templates WYSIWYG in the UI. Agents edit the same templates as WCL through the CLI (brief, D14).

### Content blocks

A content field holds a sequence of blocks. You edit them WYSIWYG, and TheVault stores each block as a structured WCL block (D34).

- **Text.** Headings, paragraphs, lists, quotes and the other typography found in Notion-style editors. Inline formatting covers bold, italic, underline, strikethrough, inline code, highlight, text colour, links and mentions of other notes. A mention is a reference and shows up in reference queries. The syntax is in the appendix (D34, D41).
- **Image** and other asset blocks.
- **Code.** Literal text with syntax highlighting for a chosen language.
- **Terminal.** A sequence of commands, each followed by its expected stdout and stderr, rendered as a terminal session.
- **Mermaid.** A diagram rendered from Mermaid source.
- **Ink.** A drawing surface for stylus, touch or mouse input.
- **Query view.** Runs a query over the vault and renders the result as a table, cards, list, details, kanban, calendar, timeline, map or canvas. A block keeps several saved views, each with its own query and view type.
- **Kanban.** A taxonomy field sets the columns. Dragging a card to another column sets that taxonomy value on the note. The field must hold a single value (D7).
- **Calendar and timeline.** These place query results by their date, datetime or date range fields.
- **Map.** Shows query results as pins at their location fields. A map uses real-world tiles by default. You can use a custom image as the map instead and still place pins on it.
- **Canvas.** A free-form board of cards, text, links and groups, stored in TheVault's own WCL canvas format (D15). Canvas nodes can show the results of a query. You can embed a canvas in content or put one on a type's template.

### Queries

- One text query language is used everywhere: in query blocks, the API, the CLI and wscript (D8).
- Its expressions use wscript syntax, for example `from task where status == "done" && due < today() sort due desc` (D27). The full grammar is in the appendix (D42).
- The language covers filtering, sorting, grouping for kanban, limits, following references, taxonomy hierarchy, and full-text match (D16, D27).
- A visual builder in the UI edits the same text query, so a query written by an agent opens in the builder and a query built in the UI can be read by an agent (D8).
- Every query result is paginated (D33).

### Search

- Full-text search covers field values and content blocks, and the query language can use it (D16).

### Automation

- Each type can have event handlers written in wscript (brief, D9).
- `before_create`, `before_update` and `before_delete` run during the save. They can change the note or reject the save (D9).
- `after_create`, `after_update` and `after_delete` run from a queue after the save, for side effects (D9).
- Handlers can query the vault and edit other notes. Those edits go through the normal save, so their own handlers run (D30).
- TheVault stops chains of handlers that trigger each other at a fixed depth and detects cycles (D9).
- History records a handler's edits as made by that handler (D30).
- Only `after_*` handlers and scheduled tasks can make HTTP calls (D30).
- Scheduled tasks run wscript on a cron schedule (D10).
- A task reads secrets by name from an encrypted store. Secrets never appear in vault files (D10).
- Each task lists the domains it may call over HTTP. Calls to any other domain fail (D10).
- The UI shows each task's run history and logs. Failed runs retry with backoff, and the UI shows failures (D10).
- Reminders and notifications are not built in. A scheduled task can query for due items and call an outside service such as ntfy (D35).

### History and deletion

- TheVault keeps every version of every note in its own version store, not in git. You can view a note's history and restore any version (D13).
- History is kept forever by default. You can turn on pruning by age or by count (D25).
- A deleted note goes to a trash, where you can restore it (D13).

### API and CLI

- An HTTP API exposes every action the UI can take, including defining types, templates, taxonomies, handlers and tasks (brief).
- The API is the same for every client. Later native mobile and desktop clients will use it without server changes (D18).
- Every write carries the version of the note it was based on. If the note has changed since that version, the write fails with a conflict (HTTP 409), and the UI shows the conflict to you (D12).
- A write returns the canonical WCL that TheVault stored (D21).
- The `vault` CLI is a client of the HTTP API (brief, D32). It:
  - prints JSON from every command.
  - reads and writes notes and definitions as raw WCL.
  - writes batches from files, with `--dry-run` showing exactly what the real run would do.
  - creates or updates notes matching a key field (upsert).
  - uploads assets.
  - runs queries and returns the results.
  - lists types and their schemas, so an agent can discover the vault's structure.
- TheVault ships an agent skill document that teaches an AI agent to use the CLI (D32).
- TheVault has no built-in importers. Agents import from other tools through the CLI (D17).
- `vault init` creates an empty vault. `vault init --starter` adds Task, Project, Person and Note types, a Status taxonomy, and a dashboard note with kanban and calendar views (D37).
- `vault reindex` rebuilds the index from the files and reports any file that fails validation (D28).
- `vault backup` writes one archive of the vault files and the app state (D25).

### Storage

- The vault is a folder of WCL files, and those files are the source of truth (D2).
- TheVault uses WCL only as a data format, with no expressions and no imports. It rejects any submitted WCL that contains them. wscript lives in separate `.ws` files (D20).
- TheVault formats every file the same way on every write, and comments are not kept (D21).
- Dates and datetimes are ISO 8601 strings, URLs are strings, and references are note ULIDs. TheVault validates them against the schema (D23).
- The folder layout is fixed (D22):

  | Path | Holds |
  |------|-------|
  | `notes/<type>/<ULID>.wcl` | One note, with its content blocks inline |
  | `types/<type>.wcl` | A type's schema and templates |
  | `taxonomies/*.wcl` | Taxonomies |
  | `scripts/*.ws` | wscript handlers and migrations |
  | `tasks/*.wcl` | Scheduled task definitions |
  | `assets/<ULID>/<filename>` | Uploaded files and images |
  | `.vault/index.db` | Search and query index, rebuilt from the files |
  | `.vault/state.db` | History, encrypted secrets, API tokens and task run logs |

- Restoring the vault files without `state.db` loses history, secrets and tokens. Back up both with `vault backup` (D25).
- Edit the vault through the UI, API or CLI only. TheVault does not watch the folder, and a direct edit skips validation, handlers, history and conflict checks until you run `vault reindex` (D28).
- TheVault does not use git. You can keep the vault folder in git yourself if you ignore `.vault/` (D24).

### Access and security

- The browser logs in with a password and keeps a session cookie (D11).
- Agents use API tokens with one of three scopes (D11):
  - `read` can only read.
  - `write` can change notes.
  - `admin` can also change types, templates, taxonomies, scripts and tasks.
- TheVault is built for a home network or tailnet. You can expose it to the internet behind a reverse proxy (D11).
- wscript has no memory limit, and its instruction limit does not count time spent in HTTP calls. TheVault adds its own time limits, response size limits and the domain allowlist (notes/01, D10).
- Scripts get no access to the server's filesystem or processes (notes/01).

### Clients

- TheVault is a responsive web app that you can install as a PWA. It takes stylus input for ink (D18).
- It needs a network connection and has no offline mode (D18).
- It supports the last two versions of Chrome, Firefox and Edge, and Safari 17.4 or later (D36).

## Success criteria

Measured on a vault of 1M notes (D31, D33), on the reference machine: AMD Ryzen 9 9950X (16 cores, 32 threads), 186 GiB RAM, and a Samsung 990 EVO Plus 4 TB NVMe drive with btrfs (D39).

| Measure | Target |
|---------|--------|
| Query on indexed fields, p95 | under 500 ms |
| Full-text search, p95 | under 1 s |
| Save, excluding handlers, p95 | under 200 ms |
| Open a note, p95 | under 300 ms |
| Full `vault reindex` | under 30 minutes |

No view loads a whole result set. Tables, lists, cards, kanban columns, calendars, timelines, map pins and canvas query nodes all paginate or virtualise (D33).

The release is done when every feature in this document is implemented and you sign it off as ready for use (D40). The sign-off covers these judgement calls:

- Every UI action has an API and CLI equivalent. Check by going through every UI action and finding its matching command.
- An agent with only the bundled skill document can discover the types, import a batch, and fix its own validation errors without human help.

## Constraints

- **Stack.** Rust backend and SolidJS 1.9 frontend. Solid 2.0 is a release candidate that splits the library ecosystem, so the move waits until it is stable (brief, D19).
- **Storage and scripting.** WCL for storage, and wscript for handlers, tasks and migrations (brief).
- **Editor.** TipTap on ProseMirror. BlockNote is ruled out because its XL packages are GPL-3.0, and Lexical because its ecosystem is built around React (D29).
- **Maps.** OpenLayers, which supports real-world tiles and custom image maps in its current release. Leaflet 2.0 is still alpha, and MapLibre has not yet released image-map support (D29).
- **Map tiles.** OpenFreeMap by default, with a configurable tile URL so you can self-host. The map UI shows OpenStreetMap attribution (D29).
- **Ink.** Built on perfect-freehand. tldraw is ruled out because it needs a paid production licence, and Excalidraw because it is React-only (D29).
- **Code blocks.** CodeMirror 6 for editing (D29).
- **Calendar.** The MIT-licensed core of FullCalendar. Its timeline views are paid, so timelines use vis-timeline (D29).
- **Mermaid 12.** It sets the Safari 17.4 floor (D36).
- **Single release.** Everything in this document ships together. There are no phases (D1).

## Non-goals

- More than one user, sharing, or real-time collaboration.
- Native mobile or desktop clients. They come later through the same API (D18).
- Offline use (D18).
- Built-in importers for Notion, Capacities, Obsidian or any other tool (D17).
- JSON Canvas import or export (D26).
- Git integration (D24).
- Watching the vault folder for direct edits (D28).
- Built-in reminders or push notifications (D35).
- wdoc, or any use of WCL beyond data storage (D20).
- The Solid 2.0 migration (D19).

## Delivery order

Everything ships in one release (D1), but the parts depend on each other. Building in this order means each step can be tested on the steps before it:

1. Storage, types and fields, validation, the index, and the API with write conflicts.
2. The CLI and the agent skill document, so agents can load test data from here on.
3. References and relationships, taxonomies, schema changes, history and trash.
4. The query language, full-text search, and table, list, card and detail views.
5. The block editor with text, image, code, terminal and Mermaid blocks, then templates with tabs.
6. Event handlers, then scheduled tasks with secrets and the HTTP allowlist.
7. Kanban, calendar and timeline views, and the visual query builder.
8. Maps, ink and canvas.
9. Login, token scopes, PWA install, `vault init --starter` and `vault backup`.

## Risks

- **Scope.** One release holds a typed object store, a block editor with nine kinds of view, a scripting engine and a scheduler. This is the biggest risk to the project (D1).
- **Early-stage dependencies.** WCL and wscript are both pre-1.0 and maintained by one person. Breaking changes are likely (notes/01).
- **Canonical formatting.** Every write reformats the file, so any layout or comments an agent writes by hand are lost (D21).
- **Atomic multi-file writes.** Two-way links, cascades and schema changes each change many files. A crash part way through must not leave half a relationship behind (D4, D6).
- **Custom components.** The editor needs a way to host Solid components inside ProseMirror blocks, and the ink and canvas surfaces are built from scratch. These are the largest frontend builds in the project (notes/03).

## Appendix A: inline markup

Paragraph strings and other text blocks use this inline syntax (D41). Structure such as headings and lists is a WCL block, not markup.

| Markup | Renders as |
|--------|------------|
| `**text**` | Bold |
| `*text*` | Italic |
| `++text++` | Underline |
| `~~text~~` | Strikethrough |
| `` `text` `` | Inline code |
| `==text==` | Highlight |
| `{red}text{/}` | Text in a colour from a fixed palette |
| `[label](https://example.com)` | Link |
| `[[01JB8Z4K2M]]` | Mention of a note, shown with the note's current title |
| `[[01JB8Z4K2M\|label]]` | Mention of a note, shown with your label |
| `\*` | A literal `*`. A backslash escapes any markup character |

A mention without a label always shows the target's current title, so renaming a note never rewrites the notes that mention it.

## Appendix B: query grammar

A query is a `from` clause followed by optional clauses, always in this order (D42):

```
from <type>[, <type>...]      or   from *
where <expression>
search "<text>"
sort <field> [asc|desc][, <field> [asc|desc]...]
group by <field>
select <field>[, <field>...]
limit <n>
```

Expressions use wscript syntax: `==`, `!=`, `<`, `<=`, `>`, `>=`, `&&`, `||`, `!`, and closures written `|x| expr`. A dot follows a single reference, so `project.status` reads the status of the note in the `project` field.

Queries add these functions to wscript:

| Function | Returns |
|----------|---------|
| `today()`, `now()` | The current date or datetime |
| `days(n)` | A duration of `n` days, for date arithmetic |
| `has(list, value)` | True if a list field contains `value` |
| `under(field, "term")` | True if a taxonomy field holds `term` or a term below it |
| `any(list, \|x\| expr)` | True if `expr` is true for any item |
| `all(list, \|x\| expr)` | True if `expr` is true for every item |
| `linked_from(type, field)` | Notes of `type` whose `field` references this note |

`this` is the note whose template or content holds the query. A Project template can list its open tasks with:

```
from task where project == this && !done sort due asc
```

Two more examples:

```
from task where under(status, "Active") && due < today() + days(7) group by status
from book, article search "event sourcing" sort updated desc limit 20
```

`limit` caps the result. Paging through a result uses a cursor in the API and CLI and is not part of the grammar (D33).
