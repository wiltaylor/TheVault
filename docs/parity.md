# UI, API and CLI parity

This is the release sign-off checklist for D40. It covers the PRD requirement that every UI action has an API and a CLI equivalent (D32). It has three parts:

1. Every UI action, with the API call it makes and the `vault` command that does the same.
2. Every route in the server's endpoint listing, with its CLI command and where the UI uses it.
3. The `vault` commands that work on a vault folder directly and have no API route.

The audit was taken on `main` at `3156cdf`. The endpoint listing is `crates/api/src/openapi.rs` (`ENDPOINTS`), and a unit test there keeps it equal to the routes `crates/api/src/lib.rs` registers. The UI actions come from reading `web/src`.

Every gap is either explained in the row where it appears or filed as a ticket. Ticket numbers appear in the **Gap** column. They are listed together at the end.

## 1. UI actions

`—` in the API column means the action is client-side and saved later by the call shown in the same group.

### Session, navigation and search

| UI action | Where | API | CLI | Gap |
|---|---|---|---|---|
| Log in | `pages/Login.tsx` | `POST /api/login` | none, see below | Explained: the CLI authenticates with a bearer token (`--token` / `VAULT_TOKEN`, D11). The password is set by `vault init` and `vault passwd` |
| Load the session's types and taxonomies | `session.ts` | `GET /api/types`, `GET /api/taxonomies` | `vault type list`, `vault taxonomy list` (or `vault schema`) | |
| Log out | `components/Shell.tsx` | `POST /api/logout` | none, see below | Explained: a CLI token has no session to end. `vault token revoke` ends a token |
| Search (header box and results page, paged) | `pages/Search.tsx` | `GET /api/search` | `vault search <text> [--cursor] [--page-size] [--all]` | |
| List the notes of a type (paged) | `pages/TypeList.tsx` | `POST /api/query` (`from <type> sort title asc select title`) | `vault query "from <type> sort title asc"` or `vault list --type <type>` | |
| Navigation, menu, home cards | `Shell.tsx`, `Home.tsx` | — | — | Client-side only |
| Set the map tile URL | `pages/Settings.tsx` | none | none | Explained: a browser preference kept in `localStorage`, not vault data (D29) |

### Notes

| UI action | Where | API | CLI | Gap |
|---|---|---|---|---|
| Create a note | `pages/TypeList.tsx` (New note), `NoteView.tsx` at `/notes/new?type=` | `POST /api/notes` | `vault put --type <type>` | Closed by #39 |
| Open a note | `pages/NoteView.tsx` | `GET /api/notes/{id}` | `vault get <id>` | |
| Save a note (fields, content, ink, canvas) | `NoteView.tsx` | `PUT /api/notes/{id}` with `base_version` | `vault put <id> --base <version>` | |
| Conflict view: save my version | `components/ConflictView.tsx` | `GET /api/notes/{id}`, then `PUT` at the stored version | `vault get <id>`, then `vault put <id> --base <stored version>` (the CLI prints the 409 body) | |
| Conflict view: keep the stored version | `ConflictView.tsx` | — | — | Client-side only |
| Delete a note | `NoteView.tsx` | `DELETE /api/notes/{id}?base_version=` | `vault delete <id> --base <version>` | |
| Reference picker and mention search | `components/FieldInput.tsx`, `editor/MentionPicker.tsx` | `GET /api/search`, or `POST /api/query` with `search "…"` | `vault search`, `vault query` | |
| Show reference and mention titles | `editor/titles.ts` | `GET /api/notes/{id}` | `vault get <id>` | |
| Choose an existing asset for an asset field | `FieldInput.tsx` | `GET /api/assets` | `vault asset list` | |
| Place or drag a location pin, edit ink, edit a canvas field | `FieldInput.tsx`, `ink/InkSurface.tsx`, `canvas/CanvasSurface.tsx` | — then `PUT /api/notes/{id}` | `vault put <id>` | |
| Open history | `pages/History.tsx` | `GET /api/notes/{id}/history` | `vault history <id>` | |
| View an earlier version | `History.tsx` | `GET /api/notes/{id}/history/{version}` | `vault get <id> --version <version>` | |
| Restore an earlier version | `History.tsx` | `POST /api/notes/{id}/restore` | `vault restore <id> <version> --base <version>` | |
| List the trash | `History.tsx` (`Trash`) | `GET /api/trash` | `vault trash list` | |
| Restore from the trash | `History.tsx` (`Trash`) | `POST /api/trash/{id}/restore` | `vault trash restore <id>` | |
| Open today's daily note, or another day's | `DailyNotes.tsx` | `POST /api/daily/{date}` (today), `GET /api/daily/{date}` (other days) | `vault daily open <date>`, `vault daily get <date>` | |
| Create the daily note for another day | `DailyNotes.tsx` (Create note for this day) | `POST /api/daily/{date}` | `vault daily open <date>` | |
| Show the pinned tree in the sidebar | `Shell.tsx`, `pins.ts` | `GET /api/pins` | `vault pin list` | |
| Pin a note to the sidebar | `NoteView.tsx` (Pin button) | `PUT /api/pins/{id}` | `vault pin add <id>` | |
| Unpin a note | `NoteView.tsx` (Unpin button), `Shell.tsx` (Pinned ⋯ menu, Unpin) | `DELETE /api/pins/{id}` | `vault pin remove <id>` | |
| Create a pinned folder | `Shell.tsx` (Pinned, New folder) | `POST /api/pins/folders` | `vault pin folder add <name> [--parent <id>]` | |
| Rename a pinned folder | `Shell.tsx` (double-click a folder, or ⋯ menu, Rename) | `PUT /api/pins/folders/{id}` | `vault pin folder rename <id> <name>` | |
| Delete a pinned folder | `Shell.tsx` (⋯ menu, Delete folder) | `DELETE /api/pins/folders/{id}` | `vault pin folder remove <id>` | |
| Move a pinned note or folder | `Shell.tsx` (drag and drop, or ⋯ menu, Move to…) | `POST /api/pins/move` | `vault pin move <id> [--parent <id>] [--index <n>]` | |
| List flagged notes | None. The page was removed from the UI at the owner's request | `GET /api/flagged` | `vault flagged` | Owner decision: API and CLI only |

### Content blocks and assets

| UI action | Where | API | CLI | Gap |
|---|---|---|---|---|
| Edit text, headings, lists, quotes, code (shell commands and output included), Mermaid, ink and query-view blocks | `editor/ContentEditor.tsx`, `blocks/` | — then `PUT /api/notes/{id}` | `vault put <id>` (as WCL or `--json`) | |
| Insert or replace an image or file | `blocks/MediaBlocks.tsx` | `POST /api/assets?filename=` | `vault asset upload <path> [--filename]` | |
| Show an image or download an asset | `MediaBlocks.tsx`, `canvas/CanvasNodes.tsx` | `GET /api/assets/{id}` | `vault asset get <id> --out <file>` | |
| Show an asset block's name and size | `MediaBlocks.tsx` | `GET /api/assets` | `vault asset list` | |
| Delete an asset, taking it out of the notes that use it after confirming | `pages/Assets.tsx` | `DELETE /api/assets/{id}[?unlink=true]` | `vault asset delete <id> [--unlink]` | |

### Query views

| UI action | Where | API | CLI | Gap |
|---|---|---|---|---|
| Run a query view, page through table, list, cards and details | `views/QueryView.tsx` | `POST /api/query` with `this`, `cursor`, `page_size` | `vault query <q> --this <id> [--cursor] [--page-size] [--all]` | |
| Details card | `QueryView.tsx` | `GET /api/notes/{id}` | `vault get <id>` | |
| Kanban column load and "more" | `views/KanbanView.tsx` | `POST /api/query` | `vault query` | |
| Kanban drag to another column | `KanbanView.tsx` | `GET /api/notes/{id}`, then `PUT /api/notes/{id}`. A 409 opens the conflict view | `vault get <id>`, then `vault put <id> --base <version>` | |
| Calendar and timeline window load | `views/DateViews.tsx` | `POST /api/query` | `vault query` | |
| Calendar and timeline item click | `DateViews.tsx` | — | — | Navigation only. Neither view writes on drag, and the PRD does not ask for drag writes there |
| Drag a map pin | `map/MapView.tsx` | `GET /api/notes/{id}`, then `PUT /api/notes/{id}` | `vault get <id>`, then `vault put <id> --base <version>` | **#44**: a 409 reverts the pin with no conflict view |
| Canvas query view kind | `QueryView.tsx` | — | — | **#42**: the view shows "not available yet" |
| Edit a query view's settings and views | `editor/QueryViewSettings.tsx` | — then the note or template `PUT` | `vault put <id>` / `vault template put` | |
| Visual builder: text to model | `editor/QueryEditor.tsx` | `POST /api/query/parse` | `vault query-parse <q>` | |
| Visual builder: model to text | `QueryEditor.tsx` | `POST /api/query/text` | `vault query-text` | |
| Views refresh after a save | `QueryView.tsx` | — | — | **#43**: query views do not re-run after a note write |

### Canvas

| UI action | Where | API | CLI | Gap |
|---|---|---|---|---|
| Add, move, resize, connect, colour or delete nodes and groups; edge arrows and captions; canvas height; zoom | `canvas/CanvasSurface.tsx` | — then the note or template `PUT` | `vault canvas put <id> <name> --base <version>` or `vault put <id>` | |
| Edit a card's content blocks in place | `CanvasSurface.tsx`, `editor/ContentEditor.tsx` | — then the note or template `PUT`; `POST /api/assets` for an image or file block | `vault canvas put` or `vault put`; `vault asset upload` | |
| Add a note node | `CanvasSurface.tsx` | `GET /api/search` | `vault search` | |
| Note node and card contents, query views in cards | `canvas/CanvasNodes.tsx` | `GET /api/notes/{id}`, `POST /api/query` | `vault get`, `vault query` | |

### Types, templates and taxonomies

| UI action | Where | API | CLI | Gap |
|---|---|---|---|---|
| Open the type editor | `pages/TypeEditor.tsx` | `GET /api/schema`, `GET /api/scripts` | `vault schema`, `vault script list` | |
| Create a type | `TypeEditor.tsx` | `PUT /api/types/{name}` | `vault type put <name> -f <file>` | |
| Preview a change to a type (renames, migration) | `TypeEditor.tsx` | `POST /api/types/{name}/change` | `vault type change <name> -f <file> [--rename old=new] [--migration <script>] [--function <fn>]` | |
| List the notes a refused plan affects | `TypeEditor.tsx` | `POST /api/query` | `vault query` | |
| Apply a type change | `TypeEditor.tsx` | `POST /api/types/{name}/change` with `plan` | `vault type change <name> … --apply <plan token>` | |
| Delete a type | `TypeEditor.tsx` | `DELETE /api/types/{name}` | `vault type delete <name>` | |
| Open the template editor | `pages/TemplateEditor.tsx` | `GET /api/schema` | `vault schema` or `vault template list <type>` | |
| Save a template | `TemplateEditor.tsx` | `PUT /api/types/{type}/templates/{name}` | `vault template put <type> <name> -f <file>` | |
| Delete a template | `TemplateEditor.tsx` | `DELETE /api/types/{type}/templates/{name}` | `vault template delete <type> <name>` | |
| Open a taxonomy | `pages/TaxonomyEditor.tsx` | `GET /api/taxonomies` | `vault taxonomy list` or `vault taxonomy get <name>` | |
| Save a taxonomy, with term renames and moves | `TaxonomyEditor.tsx` | `PUT /api/taxonomies/{name}` with `renames` | `vault taxonomy put <name> -f <file> [--rename old=new]` | |
| Delete a taxonomy | `TaxonomyEditor.tsx` | `DELETE /api/taxonomies/{name}` | `vault taxonomy delete <name>` | |

### Scripts, tasks, secrets and tokens

| UI action | Where | API | CLI | Gap |
|---|---|---|---|---|
| List scripts | `pages/Scripts.tsx` | `GET /api/scripts` | `vault script list` | |
| Open a script | `Scripts.tsx` | `GET /api/scripts/{name}` | `vault script get <name>` | |
| Create or save a script, and see compile problems | `Scripts.tsx` | `PUT /api/scripts/{name}` | `vault script put <name> -f <file>` | |
| Delete a script | `Scripts.tsx` | `DELETE /api/scripts/{name}` | `vault script delete <name>` | |
| List tasks | `pages/Tasks.tsx` | `GET /api/tasks` | `vault task list` | |
| Open a task with its runs | `Tasks.tsx` | `GET /api/tasks/{name}`, `GET /api/tasks/{name}/runs?limit=` | `vault task get <name>`, `vault task history <name> --limit <n>` | |
| Pick a task's script and secrets | `Tasks.tsx` | `GET /api/scripts`, `GET /api/secrets` | `vault script list`, `vault secret list` | |
| Create, save or pause a task | `Tasks.tsx` | `PUT /api/tasks/{name}` | `vault task put <name> -f <file>` | |
| Run a task now | `Tasks.tsx` | `POST /api/tasks/{name}/run` | `vault task run <name>` | |
| Delete a task | `Tasks.tsx` | `DELETE /api/tasks/{name}` | `vault task delete <name>` | |
| Read a run's log | `Tasks.tsx` | in the runs list | `vault task history <name>` or `vault task show-run <name> <id>` | |
| List secrets | `pages/Access.tsx` | `GET /api/secrets` | `vault secret list` | |
| Set or replace a secret | `Access.tsx` | `PUT /api/secrets/{name}` | `vault secret set <name>` (value on stdin) | |
| Delete a secret | `Access.tsx` | `DELETE /api/secrets/{name}` | `vault secret delete <name>` | |
| List tokens | `Access.tsx` | `GET /api/tokens` | `vault token list` | |
| Create a token | `Access.tsx` | `POST /api/tokens` | `vault token create <name> --scope <scope>` | |
| Revoke a token | `Access.tsx` | `DELETE /api/tokens/{name}` | `vault token revoke <name>` | |

## 2. Every route in the endpoint listing

The listing has 69 routes. `/api/test/clock` is left out of the listing on purpose: it exists only when a test build runs with `--test-clock` (S7).

| Route | CLI | UI | Note |
|---|---|---|---|
| `GET /api/health` | `vault health` | not used | Liveness check for clients and monitors |
| `GET /api/openapi.json` | none | not used | Explained: the machine-readable listing for API clients. `vault commands` is the CLI's own listing |
| `POST /api/login` | none | Log in | Explained in part 1 |
| `POST /api/logout` | none | Log out | Explained in part 1 |
| `GET /api/schema` | `vault schema` | Type and template editors | |
| `GET /api/notes` | `vault list [--type]` | not used | The UI lists notes through `POST /api/query` |
| `POST /api/notes` | `vault put --type` | none | **#39** |
| `GET /api/notes/{id}` | `vault get` | Note view and others | |
| `PUT /api/notes/{id}` | `vault put <id>` | Note save, kanban, map | |
| `DELETE /api/notes/{id}` | `vault delete` | Note delete | |
| `GET /api/notes/{id}/history` | `vault history` | History | |
| `GET /api/notes/{id}/history/{version}` | `vault get --version` | History | |
| `POST /api/notes/{id}/restore` | `vault restore` | History | |
| `GET /api/notes/{id}/canvas/{name}` | `vault canvas get` | not used | Explained: the UI already holds the canvas in the note it opened |
| `PUT /api/notes/{id}/canvas/{name}` | `vault canvas put` | not used | Explained: the UI saves canvases inside the note `PUT`, through the same writer |
| `POST /api/batch` | `vault batch [--dry-run]` | not used | Agent and import path (D32). The PRD has no batch UI |
| `POST /api/upsert` | `vault upsert [--dry-run]` | not used | Agent and import path (D32) |
| `GET /api/query` | `vault query` (sends `POST`) | not used | Explained: the same query as a URL, for links and simple clients |
| `POST /api/query` | `vault query` | Lists and views | |
| `POST /api/query/parse` | `vault query-parse` | Visual builder | |
| `POST /api/query/text` | `vault query-text` | Visual builder | |
| `GET /api/search` | `vault search` | Search and pickers | |
| `GET /api/trash` | `vault trash list` | Trash | |
| `POST /api/trash/{id}/restore` | `vault trash restore` | Trash | |
| `GET /api/daily/{date}` | `vault daily get` | Daily notes | |
| `POST /api/daily/{date}` | `vault daily open` | Daily notes | |
| `GET /api/pins` | `vault pin list` | Sidebar, Pinned | |
| `PUT /api/pins/{id}` | `vault pin add` | Note page, Pin | |
| `DELETE /api/pins/{id}` | `vault pin remove` | Note page, Unpin; sidebar ⋯ menu, Unpin | |
| `POST /api/pins/folders` | `vault pin folder add` | Sidebar, Pinned, New folder | |
| `PUT /api/pins/folders/{id}` | `vault pin folder rename` | Sidebar, rename a folder | |
| `DELETE /api/pins/folders/{id}` | `vault pin folder remove` | Sidebar, delete a folder | |
| `POST /api/pins/move` | `vault pin move` | Sidebar, drag and drop and Move to… | |
| `GET /api/types` | `vault type list` | Session load | |
| `GET /api/types/{name}` | `vault type get` | not used | The UI reads the type from `GET /api/schema` |
| `PUT /api/types/{name}` | `vault type put` | Create a type | |
| `DELETE /api/types/{name}` | `vault type delete` | Type editor | |
| `POST /api/types/{name}/change` | `vault type change` | Type editor | |
| `GET /api/flagged` | `vault flagged` | Not used. Removed from the UI at the owner's request | |
| `GET /api/types/{name}/templates` | `vault template list` | not used | The UI reads templates from `GET /api/schema` |
| `GET /api/types/{name}/templates/{template}` | `vault template get` | not used | Same reason |
| `PUT /api/types/{name}/templates/{template}` | `vault template put` | Template editor | |
| `DELETE /api/types/{name}/templates/{template}` | `vault template delete` | Template editor | |
| `GET /api/taxonomies` | `vault taxonomy list` | Taxonomy editor, session load | |
| `GET /api/taxonomies/{name}` | `vault taxonomy get` | not used | The UI reads the full list |
| `PUT /api/taxonomies/{name}` | `vault taxonomy put` | Taxonomy editor | |
| `DELETE /api/taxonomies/{name}` | `vault taxonomy delete` | Taxonomy editor | |
| `GET /api/scripts` | `vault script list` | Scripts, type and task editors | |
| `GET /api/scripts/{name}` | `vault script get` | Scripts | |
| `PUT /api/scripts/{name}` | `vault script put` | Scripts | |
| `DELETE /api/scripts/{name}` | `vault script delete` | Scripts | |
| `GET /api/assets` | `vault asset list` | Asset field, asset block | |
| `POST /api/assets` | `vault asset upload` | Image and file blocks, canvas | |
| `GET /api/assets/{id}` | `vault asset get` | Images, downloads | |
| `DELETE /api/assets/{id}` | `vault asset delete [--unlink]` | Assets | |
| `GET /api/tokens` | `vault token list` | Access | |
| `POST /api/tokens` | `vault token create` | Access | |
| `DELETE /api/tokens/{name}` | `vault token revoke` | Access | |
| `GET /api/secrets` | `vault secret list` | Access, task editor | |
| `GET /api/secrets/{name}` | `vault secret get` | not used | The list already carries each secret's name and times |
| `PUT /api/secrets/{name}` | `vault secret set` | Access | |
| `DELETE /api/secrets/{name}` | `vault secret delete` | Access | |
| `GET /api/tasks` | `vault task list` | Tasks | |
| `GET /api/tasks/{name}` | `vault task get` | Tasks | |
| `PUT /api/tasks/{name}` | `vault task put` | Tasks | |
| `DELETE /api/tasks/{name}` | `vault task delete` | Tasks | |
| `GET /api/tasks/{name}/runs` | `vault task history` | Tasks | |
| `GET /api/tasks/{name}/runs/{id}` | `vault task show-run` | not used | The runs list already carries each run's log |
| `POST /api/tasks/{name}/run` | `vault task run` | Tasks | |

## 3. CLI commands without an API route

The CLI adds no behaviour of its own except these, as SPEC.md allows (D25, D28, D32):

| Command | What it does |
|---|---|
| `vault commands` | Lists every command |
| `vault skill [--raw]` | Prints the bundled agent skill document |
| `vault init [--starter] [--password-stdin]` | Creates a vault folder, its key file and its password (D37, OQ-S2, OQ-S3) |
| `vault passwd` | Changes the login password (OQ-S3) |
| `vault reindex` | Rebuilds the index from the files and reports invalid files (D28) |
| `vault backup [--include-key]` | Writes one archive of the vault files and `state.db` (D25, OQ-S2) |
| `vault restore-backup` | Unpacks a backup into a new folder |

## Gaps filed

| Ticket | Gap |
|---|---|
| #39 | The web UI cannot create a note (closed) |
| #40 | Types, taxonomies and assets cannot be deleted from the UI (closed) |
| #41 | No CLI command for `POST /api/query/text` (closed) |
| #42 | The canvas query view kind is not rendered |
| #43 | Query views do not re-run after a note is saved |
| #44 | A map pin drag that hits a 409 reverts the pin with no conflict view |
| #45 | Gaps in the skill document found by the agent-only check |
| #46 | An e2e test times out under parallel load |
