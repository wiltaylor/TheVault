---
name: thevault
description: Read and write notes in a TheVault vault with the `vault` CLI. Inspect the types first, write notes as WCL or JSON, import many notes with a dry-run batch, upsert by a key field, run queries and full-text search, fix validation errors from the error JSON, and notify the user when work is done. Use whenever a task involves TheVault notes, types or queries.
---

# TheVault with the `vault` CLI

TheVault is a note app. Every note has a **type**, and the type lists the note's fields. The server checks every write against the type and refuses a note that does not fit, so learn the types before you write.

`vault` is a client of the server's HTTP API. It sends exactly what the API takes and prints exactly what the API answers.

## Before you start

`vault` needs two environment variables:

- `VAULT_URL`: the server, such as `http://127.0.0.1:8080`.
- `VAULT_TOKEN`: an API token. Ask the user for one if it is not set. A `read` token can only read, a `write` token can change notes and assets, and an `admin` token can change types too.

Check the connection:

```sh
vault health
```

## How output works

- Every command prints one JSON document on stdout. Parse it; do not scrape it.
- A command that fails exits 1 and prints the error as JSON. When the server refused, the JSON has `status`, the HTTP status, and `error`, a short code such as `invalid`, `conflict` or `not_found`.
- `vault commands` lists every command. `vault <command> --help` shows its flags.

In the examples, `$ID` stands for a note id and `$VERSION` for that note's current `version`, both taken from earlier output.

## 1. Learn the types

```sh
vault schema
vault type list
vault type get task
vault taxonomy get status
```

`vault schema` returns every type with its fields, and every taxonomy with its terms. For each field, read:

- `kind`: what the value must be. See the table below.
- `required`: a note must have a value.
- `computed`: only the server's handlers write it. Never send it.
- `multiple`: the value is a list.
- `options` for `select`, `taxonomy` for `taxonomy`, `target` for `reference`. A field declared as a list of terms (`items = "taxonomy"` in the type file) shows as a `multiple` taxonomy field with `items` set to `"taxonomy"`, and takes the same values.
- `relation` and `inverse` for `reference`: see "Two-way references" below.

Every note also has `title`, which is required. The server sets `created` and `updated` itself and ignores yours.

| Kind | Value |
|------|-------|
| `string` | Any text |
| `int`, `float` | A number |
| `bool` | `true` or `false` |
| `date` | `"YYYY-MM-DD"` |
| `datetime` | `"2026-10-20T09:30:00+11:00"`, seconds and an offset required |
| `date_range` | `{ start: "2026-10-20", end: "2026-11-01" }` |
| `url` | An absolute URL, such as `"https://example.com"` |
| `select` | One of the field's `options` |
| `taxonomy` | A term path from the taxonomy, names joined by `/`, such as `"Active/Blocked"`. When the taxonomy has `allow_new` set, a path it does not hold yet adds that term: `"Rust"` adds a top-level term, and `"Software/Rust"` adds `Rust` under the existing `Software`. A name that differs from an existing term only in case is that term |
| `reference` | The 26-character id of a note of the `target` type |
| `asset` | The id of an uploaded asset |
| `list` | A list of the field's `items` kind |
| `location` | `{ lat: -37.81, lng: 144.96 }` |
| `content` | A `content <field> { ... }` block of paragraphs, headings and lists. In JSON it is a block, not a field: see step 3 |

Write every field you care about. Do not rely on a field's `default` being filled in for you.

Every vault has two built-in types, marked `"builtin": true` in `vault type list`: `dailynote`, one note per day, and `fleeting`, quick captures waiting in the inbox (see step 8). They can be customised but not deleted.

### Two-way references

A `reference` field with `relation = "two_way"` names its `inverse`, a field on the target type that holds the other side. In the starter types, a task's `project` and a project's `tasks` are the two sides of one link. Set one side only. The server writes the other side in the same commit: give a task a `project` and the task's id is added to that project's `tasks`. A write reports the other notes it changed this way under `also`.

## 2. Read notes

```sh
vault list --type task
vault get "$ID"
vault history "$ID"
vault graph "$ID"
vault query 'from task where status == "Todo" sort due asc limit 20'
vault query 'from task sort due asc' --all
vault search "dashboard"
```

`get` returns the note as `wcl`, the stored WCL text, and as `note`, its JSON form, with the `version` to send back when you change it.

`graph` returns the notes linked to or from a note. `nodes` starts with the note itself; each other node has a `direction` of `out` (the note links to it), `in` (it links to the note) or `both`. `edges` lists every link among the nodes as `from`, `to`, `via` (`field` or `mention`) and `field`. Only the 60 most recently updated neighbours are returned unless you pass `--limit`; `truncated` counts the rest.

`list`, `query` and `search` leave archived notes out. Add `--archived include` to keep them or `--archived only` to read nothing else. In query text, `from task include archived` reads every task and `from task where archived` only the archived ones.

`list`, `query` and `search` return `rows`. Each row has the note's `id`, `type` and `title`, and its field values under `fields`. A row never holds `content` fields, so to read a note's body, `get` the note by its `id`.

A query is a `from` clause followed by optional clauses in this order: `include archived`, `where`, `search`, `sort`, `group by`, `select`, `limit`. A `where` expression uses `==`, `!=`, `<`, `<=`, `>`, `>=`, `&&`, `||` and `!`, and these functions:

| Function | True when |
|----------|-----------|
| `under(status, "Active")` | A taxonomy field holds that term or a term below it |
| `has(tags, "x")` | A list field contains the value |
| `contains(title, "plan")` | A text field, or any item of a text list, holds the text, ignoring case |
| `empty(due)` | A field has no value: missing, blank text or an empty list |
| `today()`, `now()`, `days(n)` | Date arithmetic, as in `due < today() + days(7)` |

`search "<text>"` keeps the notes whose title or text holds every word, ignoring case, and ranks them by relevance unless `sort` is given. Earlier words match whole words. The last word also matches as a prefix, so `search "meet"` finds `meeting`, while `search "meet "` with a trailing space, or a last word of one character, matches whole words only. A prefix stands for at most the 50 most frequent words it starts. `vault search` follows the same rule.

Results come a page at a time. Pass the `next` cursor back with `--cursor`, or use `--all` to fetch every page.

## 3. Write one note

A note is WCL: one `name = value` line per field, then a block for each `content` field.

```sh
vault put --type task <<'EOF'
title    = "Write the release notes"
status   = "Todo"
priority = "high"
due      = "2026-12-01"

content body {
  paragraph {
    text = "Cover the **importer** and the new types."
  }
}
EOF
```

The same kind of note as JSON, with `--json`. Plain fields go under `fields`. Each `content` field is a block under `blocks`: `kind` is `"content"`, `labels` holds the field name, and its paragraphs, headings and lists are blocks inside it. Every block has the same four keys: `kind`, `labels`, `fields` and `blocks`, each optional except `kind`.

```sh
cat > note.json <<'EOF'
{
  "fields": { "title": "Book the venue", "status": "Todo", "priority": "medium" },
  "blocks": [
    { "kind": "content", "labels": ["body"],
      "blocks": [
        { "kind": "paragraph", "fields": { "text": "Ask for **two** quotes." } }
      ] }
  ]
}
EOF
vault put --type task --json < note.json
```

To see any other block in JSON, `get` a note that has one and read its `note.blocks`.

The `text` of a paragraph, heading, list item or quote is inline markup: `**bold**`, `*italic*`, `[label](https://example.com)`, and so on. `[[<ULID>]]` or `[[<ULID>|label]]` mentions an existing note. `[[+<type>|<title>]]` is a placeholder link to a note that does not exist yet, such as `[[+task|Fix the pump]]`: the type must exist, and a `|`, `]` or `\` in the title is escaped with a backslash. A placeholder links nowhere until someone clicks it in the app, which creates the note and rewrites the placeholder as a mention. Write a placeholder when a note should exist but you are not creating it now; to link a note you have created, mention its id instead.

To change a note, send the whole note with the version you read. A note that changed since you read it is refused with `"status": 409` and `"error": "conflict"`; read it again, reapply your change, and retry.

```sh
vault put "$ID" --base "$VERSION" <<'EOF'
title    = "Write the release notes, part one"
status   = "In progress"
priority = "high"
EOF
```

Deleting moves a note to the trash, and it can come back:

```sh
vault delete "$ID" --base "$VERSION"
vault trash list
vault trash restore "$ID"
```

Archiving keeps a note but takes it out of lists, queries and search. The note stays readable and editable by id, and links to it still work. Archive instead of deleting when the user wants a note out of the way rather than gone:

```sh
vault archive "$ID" --base "$VERSION"
vault list --type task --archived only
```

`vault unarchive <id> --base <version>` brings it back, with the `version` that `archive` returned. Archiving is a normal write: it needs the current version and shows in `vault history`.

## 4. Import many notes

A batch is a JSON array of operations. Always run it with `--dry-run` first: a dry run changes nothing and prints exactly what the real run would do, item by item.

```sh
cat > batch.json <<'EOF'
[
  { "op": "create", "type": "task",
    "note": { "fields": { "title": "Draft the agenda", "status": "Todo", "priority": "low" } } },
  { "op": "create", "type": "task",
    "wcl": "title = \"Send the invites\"\nstatus = \"Todo\"\npriority = \"high\"\n" },
  { "op": "upsert", "type": "person", "key": "email",
    "note": { "fields": { "title": "Sam Sample", "email": "sam@example.com" } } }
]
EOF
vault batch --dry-run --file batch.json
vault batch --file batch.json
```

The operations are:

| `op` | Takes |
|------|-------|
| `create` | `type`, and the note as `note` (JSON) or `wcl` |
| `update` | `id`, `base_version`, and the note |
| `delete` | `id`, `base_version` |
| `upsert` | `type`, `key`, and the note |

The batch commits all or nothing. The report holds:

| Key | Meaning |
|-----|---------|
| `ok` | Every item succeeded. A real run committed exactly when this is `true` |
| `dry_run`, `committed` | Whether this was a dry run, and whether anything was written |
| `items` | One entry per operation, in order. `index` is its position, `result` is `created`, `updated`, `unchanged`, `deleted` or `error`. A successful entry has the note's `id`, `type`, `version` and `wcl`. An `error` entry has the item's error under `error` |
| `also` | Other notes the batch changed: the far side of a two-way reference, and the effects of deleting a referenced note. Each has `id`, `type`, `event` and `version` |
| `error` | `null`, or a failure that no single item caused, such as an event handler refusing the batch |
| `status` | Only on a failed batch: the HTTP status, `422`. The command then exits 1 |

Fix every `error` item, and the top-level `error` when it is not `null`, then run the dry run again until `ok` is `true`.

### Notes that refer to each other

A dry run makes up the ids of the notes it would create. The real run creates different ones, and `version` is `null` in a dry run. Never copy an id from a dry run into a note.

No item in a batch can refer to a note that another item in the same batch creates, and no two items may change the same note. So import in stages, in reference order: first the notes that others point at, then the notes that point at them. Run each stage for real, take the new ids from its `items`, and put them in the next stage. Here the first stage creates a person and a project, and the second creates a task that refers to both:

```sh
cat > first.json <<'EOF'
[
  { "op": "upsert", "type": "person", "key": "email",
    "note": { "fields": { "title": "Robin Rivera", "email": "robin@example.com" } } },
  { "op": "upsert", "type": "project", "key": "title",
    "note": { "fields": { "title": "Launch the shop", "status": "Todo" } } }
]
EOF
vault batch --dry-run --file first.json
vault batch --file first.json > first-report.json
PERSON=$(jq -r '.items[0].id' first-report.json)
PROJECT=$(jq -r '.items[1].id' first-report.json)

cat > second.json <<EOF
[
  { "op": "upsert", "type": "task", "key": "title",
    "note": { "fields": { "title": "Order the stock", "status": "Todo",
                          "assignee": "$PERSON", "project": "$PROJECT" } } }
]
EOF
vault batch --dry-run --file second.json
vault batch --file second.json
```

The second report lists the project under `also`, because the server added the task to the project's `tasks`. Upserts make every stage safe to rerun. To find the ids of an earlier stage later, query by the key, as in `vault query 'from person where email == "robin@example.com"'`.

## 5. Upsert by a key field

An upsert finds the note of the type whose `key` field holds the same value. It updates that note, or creates one when there is none. Running the same upsert twice changes nothing the second time, so an import that uses upserts can be rerun safely.

- The key is `title` or a single `string`, `int`, `url`, `date`, `datetime` or `select` field, and the note you send must have a value for it.
- The server does not keep key values unique. When more than one note of the type holds the value, the upsert is refused with `"status": 409` and `"error": "key_not_unique"`, and `ids` lists the matching notes. Choose a key that is unique in your data, such as a person's `email`. `title` works when no two notes of the type share a title.
- Two upserts in one batch may not have the same key value.
- An update sets only the fields and `content` blocks you send. Every other field keeps its value, so an upsert cannot clear a field. To clear one, `put` the whole note.

```sh
vault upsert --type person --key email <<'EOF'
title = "Alex Example"
email = "alex@example.com"
organisation = "Example Co"
EOF
vault upsert --type person --key email --dry-run <<'EOF'
title = "Alex Example"
email = "alex@example.com"
organisation = "Example Co"
EOF
```

## 6. Fix validation errors

A refused note prints the file and every problem, one per field, under `issues`:

```json
{
  "error": "invalid",
  "status": 400,
  "file": "notes/task/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA.wcl",
  "issues": [
    { "at": "status", "message": "\"Nope\" is not a term of taxonomy \"status\"" },
    { "at": "due", "message": "\"tomorrow\" is not an ISO 8601 date (YYYY-MM-DD)" }
  ]
}
```

`at` names the field. To fix it:

1. Look the field up in `vault type get <type>` and the table in step 1.
2. For a `taxonomy` field, list the allowed term paths with `vault taxonomy get <name>`. If the taxonomy has `allow_new`, a new term is fine, but its parent path must already exist. For a `select` field, use one of its `options`. For a `reference` field, find the target note's id with `vault query`.
3. Send the corrected note. Nothing was written while it was refused.

```sh
vault taxonomy get status
vault put --type task <<'EOF'
title  = "Fixed task"
status = "Todo"
due    = "2026-11-01"
EOF
```

In a batch, each failing item carries the same error under `error`, and `index` says which operation it was. The `status` and `error` at the top of a batch report are not an item's error: see step 4.

## 7. Files

```sh
echo "hello" > notes.txt
vault asset upload notes.txt
vault asset list
```

`asset upload` prints the asset's `id`. Put that id in an `asset` field, or in an `image` or `asset` content block.

`vault asset list` gives each asset its `kind` (`image`, `pdf`, `audio`, `video` or `other`), `uploaded_ms` and `used_by`, the count of notes, notes in the trash and types that use it. `--unused`, `--kind image|pdf|other` and `--search <words>` narrow the list. `vault asset usage <id>` lists those users: `notes` (with `how`, the field or the block kind naming the asset), `trashed` and `types`.

`vault asset delete <id>` is refused with `asset_in_use` while notes name the asset, and the error lists them under `notes`. Add `--unlink` to take the asset out of those notes and delete it in one commit. A note in the trash naming the asset refuses the delete with `asset_in_trash`, even with `--unlink`: restore the note and take the asset out, or leave the asset.

`vault asset cleanup --dry-run` lists every asset nothing uses, with their `count` and total `size`. `vault asset cleanup` deletes them in one commit.

A deleted asset goes to the asset trash for 30 days. `vault asset trash` lists it, and `vault asset restore <id>...` brings it back under the same id. Notes a delete with `--unlink` changed do not get the asset back: restore their earlier versions for that. `vault asset rename <id> <filename>` renames the file; notes name assets by id, so none changes.

## 8. Fleeting notes and the inbox

A fleeting note is a quick capture of text or images, of the built-in `fleeting` type. The app's Inbox lists the fleeting notes not yet processed; processing one means marking it done, turning it into a note of another type, or deleting it. Fleeting notes are ordinary notes, so the usual commands work on them. The type's fields are `body` (content), `image` (assets, a list) and `processed` (bool, `false` by default).

Capture one. Give it a short title, such as the first line of the text:

```sh
vault put --type fleeting <<'EOF'
title     = "Ask the landlord about the leak"
processed = false

content body {
  paragraph {
    text = "Ask the landlord about the leak under the sink."
  }
}
EOF
```

To capture an image, upload it with `vault asset upload` (step 7) and list the asset ids it prints in `image`, such as `image = ["01JB9A0C3D4E5F6G7H8J9K0M1N"]`.

List the inbox, newest first, and read a capture's body with `get`:

```sh
vault query 'from fleeting where !processed sort created desc'
vault get "$ID"
```

Mark one processed by sending the whole note back with `processed = true` and the version you read, as in step 3. To turn one into another kind of note, `put` the new note (copy the title and body), then `vault delete` the fleeting note. Only process captures when the user asks you to.

## 9. Notify the user

`vault notify` puts a notification under the bell in the web app. Use it to tell the user that long work has finished, or that something needs them. Clicking it opens the note given with `--note`, or the app page given with `--link`, such as `/settings/scheduled/<task>`. It needs a `write` token.

```sh
vault notify "Imported 12 invoices" --body "Two have no category yet." --note "$ID"
vault notifications --unread
vault notifications read --all
```

The title is required and at most 200 characters; the body is at most 2000, and the app shows its first two lines. A `--note` that does not exist, or is in the trash, is refused with status 400. A notification made with a token records `token:<name>` as its `source`. `vault notifications` lists them newest first with `unread`, the count of unread ones, and `next`, the cursor for `--cursor`. `vault notifications read <id>` marks one read, `vault notifications dismiss <id>` removes one, and `vault notifications clear` removes every read one. Notify only when the user would want to know; do not notify for each step of a task.

## 10. Paired devices

The desktop and Android apps pair with the server once and each get a `write` token of their own, named `device-<id>`. Only touch devices when the user asks. These commands need an `admin` token.

```sh
vault device pair-code
vault device list
```

`vault device pair-code` prints a code that works once, for ten minutes (`--ttl <seconds>` changes that), with `payload`, the `{server_url, code}` the app's QR scanner reads, and `qr_svg`, the QR code as an SVG image. `server_url` is `--url` unless `--server-url` names the address the phone reaches the server on. `vault device list` shows each device with its `settings`: `commands` the server may send it, the `contacts_write` and `sms_send` switches, `exclusions` (`{kind, value}` with kind `contact`, `number` or `thread`) and `skip_pattern`, a regular expression for messages never captured. `vault device rename <id> <name>` renames one, `vault device settings <id>` reads a JSON object of the settings to change from stdin, and `vault device revoke <id>` revokes one: its token stops working at once.

## Setting up a vault

Only when the user asks for a new vault. `init --starter` adds Task, Project, Person and Note types, a Status taxonomy, an open Tags taxonomy for the Note type's `tags`, a few example notes, and a dashboard note. `reindex` rebuilds the search index from the files and lists every file that fails validation; stop the server first.

```sh
printf '%s\n' "$VAULT_PASSWORD" | vault init ./my-vault --starter --password-stdin
vault reindex --vault ./my-vault
```
