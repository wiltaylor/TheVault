---
name: thevault
description: Read and write a TheVault vault with the `vault` CLI. Use when a task reads, writes, archives or deletes TheVault notes; runs a query or full-text search; imports many notes in a batch or upserts them by a key field; creates a note type or changes one that already has notes; creates a taxonomy, adds terms, or lays out a template with tabs and query views; writes a script, an event handler on a type or a scheduled task, runs it and reads its log; stores a secret a script reads; notifies the user; or pairs and manages devices.
---

# TheVault with the `vault` CLI

TheVault is a note app. Every note has a **type**, and the type lists the note's fields. The server checks every write against the type and refuses a note that does not fit, so learn the types before you write.

`vault` is a client of the server's HTTP API. It sends exactly what the API takes and prints exactly what the API answers.

## Before you start

`vault` needs two settings:

- `VAULT_URL`: the server, such as `http://127.0.0.1:8080`.
- `VAULT_TOKEN`: an API token. A `read` token can only read, a `write` token can change notes and assets, and an `admin` token can also change types, taxonomies, templates, scripts, tasks and secrets.

When neither is set in the environment or given as `--url` and `--token`, `vault` reads them from `~/.config/thevault/cli.env` (`$XDG_CONFIG_HOME/thevault/cli.env` when that is set). The install command from Settings > API tokens > Connect an AI agent writes that file, so on a machine it set up, `vault` needs nothing set. When `vault health` works but other commands answer 401, the token is missing or revoked: ask the user for a new install code or token. Never print the token or the contents of `cli.env`.

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
- `labels` for a list of strings: another list field holding one label per value, in the same order. Write both lists together, as a person's `phones = ["+61400000000"]` with `phone_labels = ["mobile"]`.
- `hidden`: the app's properties sidebar leaves the field out. It is still stored, written and queried like any field.

Step 11 lists every attribute a type file can give a field.

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
| `location` | `{ lat: -37.81, lng: 144.96 }`, or `{ x: 412.0, y: 188.5 }` in pixels on an image map |
| `content` | A `content <field> { ... }` block of paragraphs, headings and lists. In JSON it is a block, not a field: see step 3 |
| `dict` | A `dict <field> { key = value }` block of named values of the field's `values` kind |
| `ink`, `canvas` | A drawing or a canvas block, made in the app. `get` a note that has one to see its form |

Write every field you care about. Do not rely on a field's `default` being filled in for you.

Every vault has nine built-in types, marked `"builtin": true` in `vault type list`: `dailynote`, one note per day; `fleeting`, quick captures waiting in the inbox (see step 8); `message`, `call` and `conversation`, written from a paired phone; and `email`, `email_thread` and `event`, written by the Google sync. They can be customised but not deleted. Each server start adds back any field of a built-in type's shipped default that the vault's copy lacks, so to take a field out of sight set `hidden = true` on it rather than removing it. A vault without a `person` type gets one, because the phone and Google types refer to it.

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
| `has_ci(emails, "Ann@Example.com")` | As `has()`, but text compares ignoring case, so an address matches however it was stored |
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

A `toggle` is a summary line that the reader opens to see the blocks nested in it. `text` is the summary; `level` 1, 2 or 3 shows it as a heading, and 0 or none as plain text. Whether it is open is kept on each device, not in the note, so leave that out:

```sh
vault put --type task <<'EOF'
title  = "Plan the move"
status = "Todo"

content body {
  toggle {
    level = 2
    text  = "Packing list"

    list {
      style = "todo"

      item {
        text = "Books"
      }
    }
  }
}
EOF
```

An `admonition` is a callout box. Its `kind` sets the colour, icon and title: `note`, `info`, `tip`, `success`, `warning`, `danger`, `question` or `quote`. `title`, `colour` (a palette colour), `icon` (an icon name such as `flame`) and `collapsible = true` are optional. Its blocks nest inside it as a toggle's do. Neither holds a `columns` block.

```sh
cat > callout.json <<'EOF'
{
  "fields": { "title": "Check the lease", "status": "Todo" },
  "blocks": [
    { "kind": "content", "labels": ["body"],
      "blocks": [
        { "kind": "admonition", "fields": { "kind": "warning", "title": "Notice is **30 days**" },
          "blocks": [
            { "kind": "paragraph", "fields": { "text": "Give notice before the 1st." } }
          ] }
      ] }
  ]
}
EOF
vault put --type task --json < callout.json
```

The `text` of a paragraph, heading, list item, quote or toggle is inline markup, and so is an admonition's `title`: `**bold**`, `*italic*`, `[label](https://example.com)`, and so on. `[[<ULID>]]` or `[[<ULID>|label]]` mentions an existing note. `[[+<type>|<title>]]` is a placeholder link to a note that does not exist yet, such as `[[+task|Fix the pump]]`: the type must exist, and a `|`, `]` or `\` in the title is escaped with a backslash. A placeholder links nowhere until someone clicks it in the app, which creates the note and rewrites the placeholder as a mention. Write a placeholder when a note should exist but you are not creating it now; to link a note you have created, mention its id instead.

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

You can never empty the trash or delete a note from it forever. Only the signed-in user can, in the web app: every token, `admin` included, gets `"status": 403` and `"error": "human_only"`. Do not look for a way around it; tell the user when a note should be purged.

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
  { "op": "upsert", "type": "person", "key": "title",
    "note": { "fields": { "title": "Sam Sample", "emails": ["sam@example.com"] } } }
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
  { "op": "upsert", "type": "person", "key": "title",
    "note": { "fields": { "title": "Robin Rivera", "emails": ["robin@example.com"] } } },
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

The second report lists the project under `also`, because the server added the task to the project's `tasks`. Upserts make every stage safe to rerun. To find the ids of an earlier stage later, query by the key, as in `vault query 'from person where title == "Robin Rivera"'`, or by any field, as in `vault query 'from person where has(emails, "robin@example.com")'`.

## 5. Upsert by a key field

An upsert finds the note of the type whose `key` field holds the same value. It updates that note, or creates one when there is none. Running the same upsert twice changes nothing the second time, so an import that uses upserts can be rerun safely.

- The key is `title` or a single `string`, `int`, `url`, `date`, `datetime` or `select` field, and the note you send must have a value for it.
- The server does not keep key values unique. When more than one note of the type holds the value, the upsert is refused with `"status": 409` and `"error": "key_not_unique"`, and `ids` lists the matching notes. Choose a key that is unique in your data, such as a person's `homepage`. A list field, such as a person's `emails`, cannot be a key. `title` works when no two notes of the type share a title.
- Two upserts in one batch may not have the same key value.
- An update sets only the fields and `content` blocks you send. Every other field keeps its value, so an upsert cannot clear a field. To clear one, `put` the whole note.

```sh
vault upsert --type person --key homepage <<'EOF'
title = "Alex Example"
homepage = "https://alex.example"
emails = ["alex@example.com"]
organisation = "Example Co"
EOF
vault upsert --type person --key homepage --dry-run <<'EOF'
title = "Alex Example"
homepage = "https://alex.example"
emails = ["alex@example.com"]
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

Emails and phone messages carry the same `processed` flag (bool, `false` by default), so the user can sort out what still needs doing. A note without the field counts as unprocessed, because a missing field is false in a query. List the unprocessed ones, newest first:

```sh
vault query 'from email where !processed sort sent desc'
vault query 'from message where !processed sort sent desc'
```

When you have dealt with an email or a message for the user, such as by answering it, filing it or making a task from it, set `processed = true` on it the same way. The Gmail sync and the phone never reset it.

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

## 11. Create a type

Change types, taxonomies, templates, scripts, tasks and secrets only when the user asks. These commands need an `admin` token.

A type is a WCL file. Its name, such as `invoice`, is `[a-z][a-z0-9_]*`, and so is each field's; `title`, `created`, `updated` and `archived` are reserved. The file holds:

| Item | Meaning |
|------|---------|
| `label` | Required. The display name, singular. `plural`, `description`, `icon` and `colour` (a palette colour, as in step 13) are optional |
| `field <name> { ... }` | One field, in display order |
| `template default { ... }` | Required. The note page's layout: see step 14 |
| `handler "<event>" { ... }` | A script to run on the type's notes: see step 15 |

Each field takes `kind` (step 1's table), and optionally `label`, `description`, `required`, `default`, `computed`, `hidden` and `multiple`. A `required` field cannot also be `computed` or `hidden`. `multiple` is allowed on `select`, `taxonomy`, `reference` and `asset`. Some kinds take more:

| Kind | Items |
|------|-------|
| `select` | `options = ["low", "high"]`, required |
| `taxonomy` | `taxonomy = "<name>"`, required |
| `reference` | `target = "<type>"`, required, or `"*"` for any type. `relation`: `one_way` (the default), `two_way` with `inverse = "<field on the target>"` naming a field that names this one back, or `peer` (`multiple`, target this type). `on_delete`: `unlink` (the default), `restrict` or `cascade` |
| `list` | `items`, required: `string`, `int`, `float`, `bool`, `date`, `datetime`, `url` or `taxonomy` (then `taxonomy = "<name>"` too). With `items = "string"`, `labels = "<field>"` names the list of strings that labels its values (step 1) |
| `dict` | `values`, required: a scalar kind such as `string` |
| `date_range` | `of`: `date` (the default), `datetime` or `date_or_datetime` |
| `location` | `map`, required: `geo`, or `image` with `image = "<asset id>"` |
| `asset` | `accept`: `any` (the default) or `image` |

```sh
vault type put invoice <<'EOF'
label  = "Invoice"
plural = "Invoices"

field amount {
  kind     = "float"
  label    = "Amount"
  required = true
}

field received {
  kind = "date"
}

field body {
  kind = "content"
}

template default {
  tab "Invoice" {
    slot "title"
    slot "amount"
    slot "received"
    slot "body"
  }
}
EOF
vault type get invoice
```

The type exists when `vault type get invoice` prints it, and the answer of `type put` has an empty `warnings` list; a warning names an attribute the server does not know and ignored. A refused type answers `invalid_type` with each problem under `failures`, located by `at`. `type put` replaces only a type that has no notes. For one that has notes it answers 409 `type_in_use`: change it with step 12. `vault type delete <name>` deletes a type that has no notes.

## 12. Change a type that has notes

A change to a type that has notes is planned, then applied. The plan shows what every note would get, and applying it changes the type and the notes in one commit.

1. Write the whole new type file. `vault type get <name>` prints the current one under `wcl`.
2. Plan: `vault type change <name> -f <file>`. Read `added`, `removed`, `renamed`, `changes` (each note that would change, with its `edits`) and `flagged` (notes that would hold data the type no longer describes).
3. Apply: the same command with the same file and flags, plus `--apply <plan>`, where `<plan>` is the plan's `plan` value. The change is done when the answer has `"applied": true` and `vault type get` shows it. A type or note that changed after the plan is refused with 409 `plan_stale`; plan again.

```sh
vault put --type invoice <<'EOF'
title  = "March power bill"
amount = 182.4
EOF
vault type get invoice | jq -r .wcl > invoice.wcl
cat >> invoice.wcl <<'EOF'

field paid {
  kind     = "bool"
  required = true
  default  = false
}
EOF
vault type change invoice -f invoice.wcl > plan.json
vault type change invoice -f invoice.wcl --apply "$(jq -r .plan plan.json)"
```

- A `default` is written into every existing note, so it shows in `changes`. A field added with no `default` changes no note. A new `required` field must have a `default`.
- `--rename old=new` renames a field and moves its values. Name the field `new` in the file.
- `--migration <script>` runs that script's `migrate` function (`--function` names another) on each note, after renames and defaults. It takes the note as `{ id, type, event, version, note }` and returns the note body `{ fields, blocks }` to write, or `null` to leave the note.
- A removed field's values stay in the notes, and `vault flagged` lists those notes.

## 13. Taxonomies

A taxonomy is a tree of terms that `taxonomy` fields and lists of terms take values from. Its file holds `label` (required), `description`, `allow_new` (`true` lets notes add terms, as in step 1's table) and `term "<name>"` blocks in display order, which is also the kanban column order. A term may hold `colour`, `description` and child `term` blocks. A colour is one of `gray`, `brown`, `orange`, `yellow`, `green`, `blue`, `purple`, `pink` and `red`; a term without one takes its parent's. A taxonomy without `allow_new` needs at least one term.

`vault taxonomy put` replaces the whole file, so to add a term, write every existing term as well:

```sh
vault taxonomy put invoice_stage <<'EOF'
label = "Invoice stage"

term "Received" {
  colour = "gray"
}

term "Paid" {
  colour = "green"
}
EOF
vault taxonomy get invoice_stage | jq -r .wcl > stage.wcl
cat >> stage.wcl <<'EOF'

term "Disputed" {
  colour = "red"
}
EOF
vault taxonomy put invoice_stage -f stage.wcl
vault taxonomy get invoice_stage
```

The term is added when `vault taxonomy get` lists its `path`. A file that drops a term that notes hold is refused with 409 `term_in_use`, which lists the notes. To rename or move a term, add `--rename "Old/Path=New/Path"`: every note holding it is rewritten in the same commit. `vault taxonomy delete <name>` deletes a taxonomy that no field uses.

## 14. Templates

A template lays a type's notes out on the page. A type has at most two, and the server refuses any other name:

| Template | Used by | When missing |
|----------|---------|--------------|
| `default` | The note page and `details` views | Required |
| `card` | Cards, kanban and canvas views, map pins and canvas note nodes | The first tab of `default` |

A template takes an optional `label` and one or more tabs.

### Tabs

`tab "<title>" { ... }` is one tab of the page, in order. A template with one tab shows no tab bar, and a card shows only its first tab that shows for the note. A tab holds an optional `when`, then an ordered mix of slots and fixed blocks.

`when = "<condition>"` shows the tab only for notes that meet the condition. The condition is a `where` clause of step 2's query language, on the note shown: its fields, `title`, `created`, `updated` and `archived`, `this` for the note itself, the operators, and `has()`, `has_ci()`, `under()`, `contains()`, `empty()`, `any()`, `all()`, `today()`, `now()` and `days()`. A missing field is false, as in a query. A condition that does not parse, names a field the type lacks, follows a reference with `.`, or calls `linked_from()` is refused with `invalid_type` at the tab, such as `template card > tab "Overdue".when`. The app evaluates the condition as the note is edited, so the tab appears or disappears before a save. When the open tab is hidden, the page moves to the first tab that shows; when no tab shows, the first tab shows. A vault that uses `when` needs the apps at 0.1.10 or later, since older apps refuse it.

A field still has one slot per template, so a second tab that shows the same field uses `{{field}}` text. This card shows an invoice unpaid for over 30 days with its own tab, and every other invoice with the plain one:

```sh
vault template put invoice card <<'EOF'
template card {
  tab "Overdue" {
    when = "!paid && received < today() - days(30)"
    paragraph {
      text = "**Overdue**: {{amount}}, received {{received}}."
    }
  }

  tab "Card" {
    slot "title"
    slot "amount"
  }
}
EOF
vault template list invoice | jq -e '.templates[] | select(.name == "card") | .tabs[0].when'
```

### Slots

`slot "<field>"` places a field, or `title`, where it stands. A field has at most one slot in a template, columns included. A `label` inside the slot, as in `slot "amount" { label = "Total" }`, renames the field in this template only. A field with no slot is still stored, and the app's properties sidebar edits it.

### Fixed blocks

Any content block in a tab is shown on every note of the type. In a fixed block's `text`, an admonition's `title`, and an image's `alt` and `caption`, `{{field}}` shows that field's value, read-only.

| Block | Holds |
|-------|-------|
| `heading` | `level` (1 to 3) and `text` |
| `paragraph`, `quote` | `text` |
| `list` | `style` (`bullet`, `numbered` or `todo`) and `item { text = "..." }` blocks |
| `image` | `asset = "<asset id>"`, or in a template `field = "<asset field>"` to show each note's own image; `alt`, `caption`, `width` |
| `asset` | `asset = "<asset id>"`: a file card, or a PDF viewer |
| `code`, `mermaid` | `source`; `code` also takes `language` and `caption` |
| `columns` | 2 to 6 `column { width = 1 ... }` blocks side by side, each holding slots and fixed blocks. Columns do not nest |
| `toggle` | `text`, the summary, and optional `level` (0 to 3); the blocks it opens to show nest inside it |
| `admonition` | `kind` (`note`, `info`, `tip`, `success`, `warning`, `danger`, `question` or `quote`), optional `title`, `colour`, `icon` and `collapsible`; its blocks nest inside it |
| `control` | `field` and `kind`: a form control that edits the field in place. The kinds are `text`, `textarea`, `number`, `slider` (with `min` and `max`), `rating`, `toggle`, `checkbox`, `dropdown`, `radio`, `chips`, `calendar`, `datetime` and `time`. A kind that does not fit the field is refused with the field kinds it fits |
| `query_view` | A live query and its views: below |
| `ink`, `canvas` | A drawing or a canvas, made in the app |

### Query views

`query_view { ... }` runs one query and shows its results. It takes `title`, `query` (step 2's language, in which `this` is the note being shown), `default_view`, and one or more `view <name> { kind = "..." }` blocks, each a tab of the view. A view may take `title`, and `template` (`default` or `card`) for the views that show notes as cards.

| View `kind` | Shows | Takes |
|-------------|-------|-------|
| `table` | Rows | `columns`, the fields to show, default `["title"]` |
| `list` | One line per note | |
| `cards` | A grid of cards | |
| `details` | Each note expanded | |
| `kanban` | A column per term; dropping a card writes the field | `group`, required: a single-term taxonomy field |
| `calendar` | Notes on their dates | `date`, required: a `date`, `datetime` or `date_range` field; `mode`: `month` (the default), `week` or `day` |
| `timeline` | Notes along a time line | `date`, required, as for `calendar` |
| `map` | A pin per note | `location`, required: a `location` field |
| `canvas` | Cards on a canvas | |

### Write a template

`vault template put <type> <name>` creates or replaces one template from its `template <name> { ... }` block, on a type with notes too. Check it with `vault template get <type> <name>`:

```sh
vault template put invoice default <<'EOF'
template default {
  tab "Invoice" {
    heading {
      level = 2
      text  = "{{title}}"
    }
    columns {
      column {
        width = 2
        slot "amount" {
          label = "Total"
        }
        slot "received"
      }
      column {
        width = 1
        control {
          field = "paid"
          kind  = "toggle"
        }
      }
    }
    slot "body"
  }

  tab "Unpaid" {
    query_view {
      title = "Every unpaid invoice"
      query = "from invoice where !paid sort received asc"

      view table {
        kind    = "table"
        columns = ["title", "amount", "received"]
      }

      view calendar {
        kind = "calendar"
        date = "received"
      }
    }
  }
}
EOF
vault template put invoice card <<'EOF'
template card {
  tab "Card" {
    slot "title"
    slot "amount"
  }
}
EOF
vault template get invoice default
```

The template is saved when `vault template get` prints it. A refused template answers `invalid_type`, and each issue's `at`, such as `template card > tab "Card" > slot "nope"`, names the item to fix. `vault template list <type>` lists a type's templates, and `vault template delete <type> card` deletes any template but `default`.

## 15. Scripts and handlers

A script is a wscript file, `scripts/<name>.ws`, of Rust-like functions. A handler on a type runs one of its functions on an event of the type's notes, and a scheduled task (step 16) runs one on a schedule.

- `use <module>` at the top imports a module. A function that can fail returns `Result[T, string]`: `?` stops the run on an error, and `Err("message")` fails it with that message.
- A note is a `Value`, `{ id, type, version, note }`, where `note` is `{ fields, blocks }`. Read one with `.get("id").unwrap().as_string().unwrap()`, or `.as_int()` and `.as_bool()` for other kinds. Build one with `Value::Map(#{ "key": Value::String("text") })`, `Value::Int(2)`, `Value::Bool(true)` or `Value::Null`.
- `fmt("{} unpaid", count)` formats text, and `vault::log(text)` adds a line to the run's log.

| `vault::` function | Does |
|--------------------|------|
| `get(id)`, `query(text)`, `query_page(text, size, cursor)`, `search(text, limit)` | Read notes. A query row is `{ id, type, title, fields }`; a page is `{ rows, total, next }`, with `""` as the first cursor |
| `create(type, body)`, `update(id, body, version)`, `delete(id, version)` | Write a note through the normal write path, which runs that note's handlers too |
| `patch(id, fields, version)` | Set some fields and keep the rest. `Value::Null` removes a field, and a `version` of `""` takes the note as it is now |
| `append_text(id, field, text, version)` | Append paragraphs to a content field |
| `daily(date)` | The daily note of `YYYY-MM-DD`, or of today for `""`, created when missing |
| `notify(title, body, note)` | Notify the user, as in step 9. `""` for no body or no note |
| `today()`, `now()` | Today as `YYYY-MM-DD` (in a task's `timezone`, else UTC), and now in milliseconds |

The modules a run may use depend on what runs it. A script that uses a module its run lacks does not compile:

| Run | `vault` | `http` | `secrets` | `fs` |
|-----|---------|--------|-----------|------|
| `before_*` handler, migration | yes, without `notify`, `daily`, `restore` or asset uploads | no | no | no |
| `after_*` handler | yes | yes | yes | no |
| Scheduled task | yes | yes | yes | yes |

`http::get(url, headers)` and `http::post(url, headers, body)` return `{ status, headers, body }` and reach only the hosts in the run's `allow_domains`. `json::parse(text)` reads JSON. A run stops after its time limit (a task's `timeout`, 30 seconds by default) or 50 million instructions, and `vault::query` stops at 10,000 rows.

A handler is a block in the type file, `handler "<event>" { script = "<script>" }`. It calls the function named after the event unless `function` names another, and an `after_*` handler takes `allow_domains` and `secrets` lists as a task does. Handlers of one event run in file order. The function takes the note as a `Value`, with `version` null on a create:

| Event | Runs | Return |
|-------|------|--------|
| `before_create`, `before_update` | Inside the save | A note body to save in place of the note, `null` to keep it, or `Err` to refuse the save |
| `before_delete` | Inside the delete | `Err` to refuse it |
| `after_create`, `after_update`, `after_delete` | From a queue, just after the commit | `Err` records a failed run |

Write the script, then add the handler with a planned change (step 12). Here every new invoice gets today's date in `received`:

```sh
vault script put invoice <<'EOF'
use vault

fn after_create(event: Value) -> Result[(), string] {
    let id = event.get("id").unwrap().as_string().unwrap()
    vault::patch(id, Value::Map(#{ "received": Value::String(vault::today()) }), "")?
    Ok(())
}
EOF
vault type get invoice | jq -r .wcl > invoice.wcl
cat >> invoice.wcl <<'EOF'

handler "after_create" {
  script = "invoice"
}
EOF
vault type change invoice -f invoice.wcl > plan.json
vault type change invoice -f invoice.wcl --apply "$(jq -r .plan plan.json)"
vault put --type invoice <<'EOF'
title  = "April water bill"
amount = 64.0
paid   = false
EOF
```

`script put` saves a script that does not compile, so check its answer: the script compiles when `problems` is `[]`, and each problem's `message` says what to fix. The handler works when the note, read again a moment after it is created, holds `received`, and `vault history <id>` lists a version by `handler invoice`. `vault script delete <name>` is refused while a handler or task names the script.

## 16. Scheduled tasks

A task runs a script's function, `run` unless `function` names another, on a schedule. Its file holds:

| Item | Meaning |
|------|---------|
| `schedule` | Required. Five-field cron: minute, hour, day of month, month, day of week. `"0 9 * * 1"` is Mondays at 09:00 |
| `script` | Required. The script name |
| `timezone` | The IANA zone the schedule and `vault::today()` use, such as `"Australia/Melbourne"`. Default `"UTC"` |
| `label`, `description` | Shown in Settings > Scheduled |
| `allow_domains` | Hosts `http` may reach. `["*"]` allows any |
| `secrets` | The secrets the script may read and write: see step 17 |
| `read_paths`, `write_paths` | Absolute folders outside the vault that `fs` may read, or read and write |
| `retries`, `retry_delay`, `timeout` | Retries after a failure (default `3`), the first retry's delay (`"PT1M"`, doubled each time), and the time limit per run (`"PT30S"`) |
| `paused` | `true` stops it firing on its schedule. It can still run by hand |

```sh
vault script put invoice_review <<'EOF'
use vault

fn run() -> Result[(), string] {
    let page = vault::query_page("from invoice where !paid", 1, "")?
    let unpaid = page.get("total").unwrap().as_int().unwrap()
    vault::log(fmt("{} unpaid invoices", unpaid))
    vault::notify("Review the invoices", fmt("{} are unpaid.", unpaid), "")?
    Ok(())
}
EOF
vault task put invoice_review <<'EOF'
label    = "Invoice review"
schedule = "0 9 * * 1"
timezone = "Australia/Melbourne"
script   = "invoice_review"
EOF
vault task run invoice_review
vault task history invoice_review --limit 5
vault notifications --unread
```

`task put` answers with `schedule_text`, such as `Mondays at 09:00 (Australia/Melbourne)`, and `next_run`: check that it says what the user asked for. `vault task run` runs the task now and prints the run when it ends; `--no-wait` prints it as soon as it starts, with `outcome` `running`, for `task show-run` to read later. It exits 0 whatever the outcome, so read the run: the task works when `outcome` is `succeeded`. A failed run has `outcome` `failed` and the error in `message`, and `log` holds what the script logged either way. `vault task history <name>` lists the runs newest first, `vault task show-run <name> <id>` prints one, and `vault task get <name>` shows the task with its `latest_run`. `vault task delete <name>` deletes a task and keeps its runs.

## 17. Secrets

A secret is a named value, such as an API key, stored encrypted on the server. No command or script can list or print a value: `vault secret list` and `vault secret get <name>` show only names and times. Set one from stdin, so the value stays out of the command line:

```sh
printf '%s' "sk-example-123" | vault secret set invoice_api_key
vault script put invoice_sync <<'EOF'
use secrets
use vault

fn run() -> Result[(), string] {
    let key = secrets::get("invoice_api_key")?
    vault::log(fmt("the key has {} characters", key.len()))
    Ok(())
}
EOF
vault task put invoice_sync <<'EOF'
schedule = "0 * * * *"
script   = "invoice_sync"
secrets  = ["invoice_api_key"]
paused   = true
EOF
vault task run invoice_sync
```

A script reads and writes only the secrets that its task, or its `after_*` handler, lists in `secrets`. `secrets::get(name)` returns the value, and `secrets::set(name, value)` replaces it, which suits state a task keeps between runs, such as a sync cursor. The task read the secret when the run's `outcome` is `succeeded`. Keep a secret's value out of logs, notifications and notes. `vault secret delete <name>` deletes one.

## The user's AI panel

The web app and the desktop and Android apps have an AI panel, where the user's own AI provider (Settings > AI) answers questions about the vault with read-only note tools and proposes edits that the user applies. An applied edit is an ordinary write by the user, so a note can change between your read and your write: send its `version` as usual, and on a 409 `conflict` read it again. Scripts reach the same providers through the `ai` module, `ai::complete` and `ai::chat`, in `after_*` handlers and tasks, without listing the AI secrets or hosts.

## Setting up a vault

Only when the user asks for a new vault. `init --starter` adds Task, Project, Person and Note types, a Status taxonomy, an open Tags taxonomy for the Note type's `tags`, a few example notes, and a dashboard note. `reindex` rebuilds the search index from the files and lists every file that fails validation; stop the server first.

```sh
printf '%s\n' "$VAULT_PASSWORD" | vault init ./my-vault --starter --password-stdin
vault reindex --vault ./my-vault
```
