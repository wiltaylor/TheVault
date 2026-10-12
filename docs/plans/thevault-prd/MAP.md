---
slug: thevault-prd
status: building
created: 2026-10-12
---

# TheVault PRD

## Destination
Confirmed 2026-10-12. A PRD exists for TheVault: a single-user self-hosted web
app (Rust backend, SolidJS frontend) where a vault holds notes that are instances
of user-defined types. Each type has a schema of typed fields (dict, list, int,
float, string, bool, taxonomy, typed reference with one-way, two-way and peer
relationships, content, asset, ink) and a WYSIWYG template that lays fields and
blocks across tabs. Content is Notion-style blocks, including query-driven
table/card/list/detail/kanban/calendar/timeline/map/canvas views with saved
views, plus mermaid, highlighted code, terminal and ink blocks. wscript event
handlers run on create/update/delete, and scheduled wscript tasks can call
external APIs. Notes are stored as WCL. Every UI action is exposed through an
HTTP API, and a CLI over that API lets AI agents read and edit the vault,
including raw WCL. The PRD covers users, scope, functional requirements per area,
non-functional requirements and phasing, not implementation design.

## Grounding
The project directory is empty. Facts come from adjacent repos and the web.
- WCL can define a schema and validate instances, and it rejects dangling refs (`notes/01-wcl-wscript.md`).
- WCL has no date, URL or binary type, and a cross-file ref resolves only via import (`notes/01-wcl-wscript.md`).
- WCL serialisation reprints the whole file and is not format-preserving. `wcl set` rewrote unrelated lines (`notes/01-wcl-wscript.md`).
- WCL expressions and imports can read the host filesystem unless loading is restricted (`notes/01-wcl-wscript.md`).
- wscript (`/home/wil/orca/wscript`, v0.2.0) is statically typed, has no async and no HTTP, caps instructions and call depth but not memory (`notes/01-wcl-wscript.md`).
- The earlier LifeOS plan is a failed experiment. The user excluded it as prior art (2026-10-12).
- ai-kg uses files in git as the source of truth with a rebuildable DB index, and one HTTP API shared by the CLI and the UI (`notes/02-prior-art.md`).
- llm-wiki and the Capacities export provide seed schemas and test data (`notes/02-prior-art.md`).
- Leaflet 2.0 is still alpha. OpenLayers supports both geo tiles and image maps today. MapLibre image-map support is unreleased (`notes/03-frontend-libs.md`).
- TipTap/ProseMirror core is framework-agnostic. BlockNote and Lexical lean React. tldraw needs a paid production licence (`notes/03-frontend-libs.md`).
- JSON Canvas spec 1.0 has no ink or viewport. FullCalendar timeline views are paid. Solid 2.0 is at RC (`notes/03-frontend-libs.md`).

## Decisions
| ID | Decision | Struck by | Recorded in |
|----|----------|-----------|-------------|
| D1 | Everything in the destination ships in one release (Q1) | | `notes/04-phasing.md` |
| D2 | WCL files on disk are the source of truth (Q2) | | `notes/05-source-of-truth.md` |
| D3 | Add date, datetime, date range, location, url, select field kinds and a computed flag (Q6) | | `notes/06-field-kinds.md` |
| D4 | Two-way and peer refs stored on both notes, written atomically (Q7) | | `notes/07-two-way-refs.md` |
| D5 | Each reference field declares `on_delete`: `unlink` (default), `restrict` or `cascade` (Q8) | | `notes/08-on-delete.md` |
| D6 | Schema changes apply eagerly with preview; new required fields need a default (Q9) | | `notes/09-schema-evolution.md` |
| D7 | Global, hierarchical taxonomies; fields declare single or multi (Q10) | | `notes/10-taxonomies.md` |
| D8 | Text query language plus visual builder, one language everywhere (Q11) | | `notes/11-query-language.md` |
| D9 | Sync `before_*` handlers can modify/reject; queued `after_*` for side effects (Q12) | | `notes/12-handler-timing.md` |
| D10 | Cron tasks with encrypted secrets, HTTP domain allowlist, run history, retries (Q13) | | `notes/13-scheduled-tasks.md` |
| D11 | Password+cookie for browser, scoped API tokens for agents, LAN-first (Q14) | | `notes/14-auth.md` |
| D12 | Optimistic versioning, stale write returns 409 (Q15) | | `notes/15-concurrency.md` |
| D13 | Internal version store with restore, plus trash (Q16) | | `notes/16-history.md` |
| D14 | Several named templates per type incl. card and detail variants (Q17) | | `notes/17-templates.md` |
| D15 | Native WCL canvas schema (Q18) | | `notes/18-canvas-storage.md` |
| D16 | Full-text search over fields and content, usable in queries (Q19) | | `notes/19-search.md` |
| D17 | No built-in importers; agents import via API/CLI (Q20) | | `notes/20-import.md` |
| D18 | Responsive PWA with stylus input, no offline (Q21) | | `notes/21-clients.md` |
| D19 | Solid 1.9 (Q22) | | `notes/22-solid-version.md` |
| D20 | Data-only WCL subset; no wdoc (Q3) | | `notes/23-wcl-dialect.md` |
| D21 | Vault owns canonical format; comments not preserved (Q4) | | `notes/24-formatting.md` |
| D22 | Fixed layout: notes by type and ULID, types/, taxonomies/, scripts/, tasks/, assets/, .vault/ (Q5) | | `notes/25-file-layout.md` |
| D23 | Dates ISO 8601, URLs strings, refs ULIDs; validated by TheVault (Q32) | | `notes/26-value-encoding.md` |
| D24 | App does not use git; user may (Q23) | | `notes/27-git.md` |
| D25 | index.db rebuildable, state.db holds history/secrets/tokens/logs; vault backup (Q24) | | `notes/28-non-file-state.md` |
| D26 | No JSON Canvas import/export (Q25) | | `notes/29-json-canvas.md` |
| D27 | Purpose-built query language with wscript expression syntax (Q26) | | `notes/30-query-syntax.md` |
| D28 | Direct file edits unsupported; vault reindex validates and reports (Q27) | | `notes/31-out-of-band-edits.md` |
| D29 | Library set: TipTap, OpenLayers, perfect-freehand, CodeMirror 6, FullCalendar core, vis-timeline; OpenFreeMap tiles (Q28) | | `notes/32-libraries.md` |
| D30 | Handlers can query and edit other notes; HTTP only in after_* and tasks (Q29) | | `notes/33-handler-host-api.md` |
| D31 | Design for 1M notes (Q30) | | `notes/34-scale.md` |
| D32 | Full agent CLI: JSON, raw WCL, batch + dry-run, upsert, assets, query, schema inspection, skill doc (Q31) | | `notes/35-agent-cli.md` |
| D33 | p95 targets at 1M notes; all views paginate or virtualise (Q33) | | `notes/36-performance.md` |
| D34 | Content blocks are structured WCL blocks with inline markup in text (Q34) | | `notes/37-content-serialisation.md` |
| D35 | No built-in reminders; scheduled tasks call external services (Q35) | | `notes/38-reminders.md` |
| D36 | Evergreen browsers, Safari 17.4+ (Q36) | | `notes/39-browser-floor.md` |
| D37 | Opt-in starter pack via vault init --starter (Q37) | | `notes/40-starter-content.md` |
| D38 | Purpose: the note app with every feature the user wants; AI access is one feature (OQ1) | | `notes/41-purpose.md` |
| D39 | Benchmark on the user's Ryzen 9 9950X / 186 GiB / NVMe machine (OQ2) | | `notes/42-benchmark-hardware.md` |
| D40 | Done = all PRD features in and signed off by the user (OQ3) | | `notes/43-release-done.md` |
| D41 | Inline markup: Markdown-like subset, `[[ULID]]` mentions, palette colours (OQ4, defaulted) | | `notes/44-inline-markup.md` |
| D42 | Query grammar: fixed clause order, wscript expressions, host functions, `this` context (OQ4, defaulted) | | `notes/45-query-grammar.md` |

## Frontier
Session boundary offered 2026-10-12 (judgement: large map, 19-question first round). User declined and ran all in one session.
- [x] Q1 (decision) — Phasing: what ships in the first usable release, and what is deferred? — settled by D1
- [x] Q2 (decision) — Source of truth: WCL files on disk (git-tracked) with a rebuildable DB index, or DB as truth with WCL as export? — settled by D2
- [x] Q3 (decision) [blocked_by: ~~Q2 (D2)~~] — WCL dialect: full WCL in note files, or a restricted data-only subset (no expressions, no imports)? — settled by D20
- [x] Q4 (decision) [blocked_by: ~~Q2 (D2)~~] — Formatting: does the vault own a canonical format (every save reformats), or does WCL gain format-preserving edits upstream? — settled by D21
- [x] Q5 (decision) [blocked_by: ~~Q2 (D2)~~] — File layout: one file per note with content blocks inline, where type schemas and templates live, where assets live. — settled by D22
- [x] Q6 (decision) — Missing field kinds: add date/datetime, location, url, and enum/select to the field list? — settled by D3
- [x] Q7 (decision) — Two-way references: stored on both notes, or stored once and the inverse derived by the index? — settled by D4
- [x] Q8 (decision) — Reference integrity: what happens to referencing notes when a target is deleted? — settled by D5
- [x] Q9 (decision) — Schema evolution: what happens to existing notes when a type's schema changes? — settled by D6
- [x] Q10 (decision) — Taxonomies: flat or hierarchical, global or per-type, single or multi-valued? — settled by D7
- [x] Q11 (decision) — Query language: text DSL, visual builder, or both, and is it the same language in blocks, API, CLI and scripts? — settled by D8
- [x] Q12 (decision) — Event handler timing: pre-commit (can modify or reject) or post-commit (side effects), and how are handler-triggered cascades bounded? — settled by D9
- [x] Q13 (decision) — Scheduled tasks: secrets storage, HTTP host API, run history and failure handling. — settled by D10
- [x] Q14 (decision) — Auth and exposure: single user, but how are the browser and agent CLI authenticated, and is it LAN-only or internet-facing? — settled by D11
- [x] Q15 (decision) — Concurrent edits between UI and agents: optimistic version check, last-write-wins, or locking? — settled by D12
- [x] Q16 (decision) — History: per-note version history / undo, via git or internal? — settled by D13
- [x] Q17 (decision) — Templates: one template per type or several selectable views; are templates shareable across types? — settled by D14
- [x] Q18 (decision) — Canvas storage: extended JSON Canvas embedded in WCL, or native WCL canvas schema? — settled by D15
- [x] Q19 (decision) — Search: full-text search across content blocks in v1? — settled by D16
- [x] Q20 (decision) — Import: Capacities/Notion/Obsidian importers in scope? — settled by D17
- [x] Q21 (decision) — Clients: desktop browser only, or mobile/PWA/offline as a requirement? — settled by D18
- [x] Q22 (decision) — Solid 1.9 or Solid 2.0? — settled by D19
- [x] Q23 (decision) — Git: does the app commit the vault to git, or is git left to the user? — settled by D24
- [x] Q24 (decision) — Non-file state: where do version history, secrets and tokens live, and how is history retained? — settled by D25
- [x] Q25 (decision) — JSON Canvas import/export for the native canvas schema? — settled by D26
- [x] Q26 (decision) — Query syntax basis: new DSL, SQL-like, or built on WCL/wscript expressions? — settled by D27
- [x] Q27 (decision) — Out-of-band edits: are direct edits to WCL files on disk watched and indexed, or unsupported? — settled by D28
- [x] Q28 (decision) — Frontend libraries and map tiles: adopt the research set (TipTap/ProseMirror, OpenLayers, perfect-freehand, CodeMirror 6, FullCalendar core, vis-timeline) and OpenFreeMap tiles? — settled by D29
- [x] Q29 (decision) — Handler host API: may handlers query the vault, edit other notes, and call HTTP? — settled by D30
- [x] Q30 (decision) — Scale: target vault size (notes, assets) for performance requirements? — settled by D31
- [x] Q31 (decision) — CLI for agents: batch writes, dry-run, JSON output, asset upload, upsert by key? — settled by D32
- [x] Q32 (decision) — Encoding of dates, URLs and references in WCL? — settled by D23
- [x] Q33 (decision) [blocked_by: ~~Q30 (D31)~~] — Performance targets at 1M notes: query, full-text, save, reindex latency? — settled by D33
- [x] Q34 (decision) — Content block serialisation: how is a content field's block tree written in WCL? — settled by D34
- [x] Q35 (decision) — Date-driven reminders: built-in notifications, or left to scheduled tasks? — settled by D35
- [x] Q36 (decision) — Browser support floor (Mermaid 12 requires Safari 17.4+)? — settled by D36
- [x] Q37 (decision) — Starter content: ship example types (from llm-wiki/Capacities shapes) in a new vault? — settled by D37

## Fog

## Accepted unknowns
- Atomic multi-file write mechanism (D4) — implementation detail, the requirement is atomicity
- Handler cascade depth limit value (D9) — any small bound works
- Token storage and rotation details (D11) — implementation detail under the scope model

- Directory sharding under notes/<type>/ at 1M notes (D22) — invisible to agents, who go through the API
- Batch write atomicity, whole batch vs per item (D32) — either works if the CLI reports per-item results and dry-run matches the real run
- Build order within the single release (D1) — /to-spec writes it from the dependency graph
- wscript sandbox gaps: no memory cap, fuel excludes host calls (notes/01) — PRD states host-side timeouts, size limits and egress controls as requirements; upstream fixes are not a dependency
- WCL/wscript are pre-1.0 single-author upstreams (notes/01) — PRD lists this as a dependency risk; pinning versions is an implementation choice
- Map engine swappability for a future MapLibre move (notes/03) — implementation concern; PMTiles self-hosting is covered by the configurable tile URL (D29)
- CRDT-compatible document model (notes/03) — collaboration is out of scope and the PRD does not constrain the model for it

## Out of scope
- Native mobile and desktop clients (D18)
- Offline mode (D18)
- Built-in importers (D17)
- Real-time multi-user collaboration (single user)
- Solid 2.0 migration (D19)
- JSON Canvas import/export (D26)
- Git integration (D24)
- wdoc and other WCL applications (D20)
