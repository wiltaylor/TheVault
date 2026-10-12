---
name: thevault
description: Read and write notes in a TheVault vault with the `vault` CLI. Inspect the types first, write notes as WCL or JSON, import many notes with a dry-run batch, upsert by a key field, run queries and full-text search, and fix validation errors from the error JSON. Use whenever a task involves TheVault notes, types or queries.
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
- `options` for `select`, `taxonomy` for `taxonomy`, `target` for `reference`.

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
| `taxonomy` | A term path from the taxonomy, names joined by `/`, such as `"Active/Blocked"` |
| `reference` | The 26-character id of a note of the `target` type |
| `asset` | The id of an uploaded asset |
| `list` | A list of the field's `items` kind |
| `location` | `{ lat: -37.81, lng: 144.96 }` |
| `content` | A `content <field> { ... }` block of paragraphs, headings and lists |

Write every field you care about. Do not rely on a field's `default` being filled in for you.

## 2. Read notes

```sh
vault list --type task
vault get "$ID"
vault history "$ID"
vault query 'from task where status == "Todo" sort due asc limit 20'
vault query 'from task sort due asc' --all
vault search "dashboard"
```

`get` returns the note as `wcl`, the stored WCL text, and as `note`, its JSON form, with the `version` to send back when you change it.

A query is a `from` clause followed by optional clauses in this order: `where`, `search`, `sort`, `group by`, `select`, `limit`. A `where` expression uses `==`, `!=`, `<`, `<=`, `>`, `>=`, `&&`, `||` and `!`, and these functions:

| Function | True when |
|----------|-----------|
| `under(status, "Active")` | A taxonomy field holds that term or a term below it |
| `has(tags, "x")` | A list field contains the value |
| `today()`, `now()`, `days(n)` | Date arithmetic, as in `due < today() + days(7)` |

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

The same note as JSON, with `--json`:

```sh
echo '{ "fields": { "title": "Book the venue", "status": "Todo", "priority": "medium" } }' \
  | vault put --type task --json
```

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

The batch commits all or nothing. The report has `ok`, and one entry in `items` per operation with its `result`: `created`, `updated`, `unchanged`, `deleted`, or `error` with the `error` that stopped it. Fix every `error` item and run the dry run again until `ok` is `true`.

## 5. Upsert by a key field

An upsert finds the note of the type whose `key` field holds the same value. It updates that note, or creates one when there is none. Running the same upsert twice changes nothing the second time, so an import that uses upserts can be rerun safely.

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
2. For a `taxonomy` field, list the allowed term paths with `vault taxonomy get <name>`. For a `select` field, use one of its `options`. For a `reference` field, find the target note's id with `vault query`.
3. Send the corrected note. Nothing was written while it was refused.

```sh
vault taxonomy get status
vault put --type task <<'EOF'
title  = "Fixed task"
status = "Todo"
due    = "2026-11-01"
EOF
```

In a batch, each failing item carries the same error under `error`, and `index` says which operation it was.

## 7. Files

```sh
echo "hello" > notes.txt
vault asset upload notes.txt
vault asset list
```

`asset upload` prints the asset's `id`. Put that id in an `asset` field, or in an `image` or `asset` content block.

`vault asset delete <id>` is refused with `asset_in_use` while notes name the asset, and the error lists them under `notes`. Add `--unlink` to take the asset out of those notes and delete it in one commit.

## Setting up a vault

Only when the user asks for a new vault. `init --starter` adds Task, Project, Person and Note types, a Status taxonomy, a few example notes, and a dashboard note. `reindex` rebuilds the search index from the files and lists every file that fails validation; stop the server first.

```sh
printf '%s\n' "$VAULT_PASSWORD" | vault init ./my-vault --starter --password-stdin
vault reindex --vault ./my-vault
```
