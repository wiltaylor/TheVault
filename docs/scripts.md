# Scripts reference

Scripts are wscript files, `scripts/<name>.ws`. They run as type handlers (`docs/vault-format.md` "Handlers"), schema-change migrations, scheduled scripts (`docs/vault-format.md` "Task files", Settings > Scheduled in the web client), and on a paired phone as device event handlers and device commands (`docs/vault-format.md` "Device files"). This page lists every host function a script can call, and the payload of every device event and command.

## What each kind of run can use

| Module | `before_*` handlers, migrations | `after_*` handlers | Scheduled scripts |
|--------|---------------------------------|--------------------|-------------------|
| `vault` | yes | yes | yes |
| `http` | no | yes | yes |
| `secrets` | no | yes | yes |
| `fs` | no | no | yes |
| `value`, `json`, `math`, `regex` | yes | yes | yes |

A script that uses a module its kind of run lacks does not compile. wscript's own `fs`, `process` and `time` modules are never available.

Each run has limits: 50 million instructions, a call depth of 256, a wall-clock time limit (a task's `timeout`, 30 seconds by default), 1 MiB per HTTP response, 16 MiB per file or asset read, 10,000 rows per `vault::query`, and 64 KiB of log.

## Conventions

- Every function that can fail returns `Result[T, string]`. Use `?` to stop the run on an error, or `match` to handle it. A run whose entry function returns `Err(message)` is recorded as failed with that message.
- Notes travel as a `Value` in the JSON form of the notes API: `{ id, type, version, note }`, where `note` is `{ fields, blocks }`.
- A `version` argument guards a change: the write is refused when the note has changed since that version. Where a function says so, an empty string `""` takes the note as it is now.
- A scheduled script's changes are recorded in note history as made by `task <name>`. A handler's are recorded as made by `handler <script>`. Every change goes through the normal write path: it is validated, a history version is kept, and the note's handlers run.

Build a `Value` with its constructors, or parse JSON:

```rust
use vault

fn run() -> Result[string, string] {
    let body = Value::Map(#{
        "fields": Value::Map(#{
            "title": Value::String("Weekly review"),
            "estimate": Value::Int(2),
        }),
    })
    let saved = vault::create("task", body)?
    Ok(saved.get("id").unwrap().as_string().unwrap())
}
```

## `vault`

### Reading notes

| Function | Returns |
|----------|---------|
| `vault::get(id: string)` | `Result[Value, string]`: the note `{ id, type, version, note }` |
| `vault::query(text: string)` | `Result[List[Value], string]`: every row of a query, as `{ id, type, title, fields }`. More than 10,000 rows is an error; add a `limit` or page |
| `vault::query_page(text: string, size: int, cursor: string)` | `Result[Value, string]`: one page, `{ rows, total, next }`. Pass `""` for the first page and `next` for the one after; `next` is null on the last page. `size` is 1 to 1000 |
| `vault::search(text: string, limit: int)` | `Result[List[Value], string]`: full-text search over every note that is not archived, best match first, at most `limit` rows |
| `vault::search_archived(text: string, limit: int, archived: string)` | `Result[List[Value], string]`: `vault::search` with archived notes left out (`"exclude"`), kept (`"include"`) or searched alone (`"only"`) |
| `vault::links(id: string)` | `Result[List[Value], string]`: the notes `id` links to, as `{ id, type, title, via, field }`. `via` is `field` for a reference field and `mention` for a `[[link]]` in text. `type` and `title` are null for a link to a missing note |
| `vault::backlinks(id: string)` | `Result[List[Value], string]`: the notes that link to `id`, in the same form |

The query language is in `docs/plans/thevault-prd/PRD.md`, Appendix B. A query leaves archived notes out unless it says `include archived` after its types or names `archived` in its `where`, as `from task where archived` does; see `docs/vault-format.md`, "Archived notes in queries".

```rust
use vault

fn run() -> Result[int, string] {
    let page = vault::query_page("from task where !done sort due asc", 100, "")?
    let open = page.get("total").unwrap().as_int().unwrap()
    for hit in vault::search("invoice", 5)? {
        vault::log(hit.get("title").unwrap().as_string().unwrap())
    }
    Ok(open)
}
```

### Changing notes

| Function | Returns |
|----------|---------|
| `vault::create(type_name: string, note: Value)` | `Result[Value, string]`: the new note. `note` is `{ fields, blocks }` |
| `vault::update(id: string, note: Value, version: string)` | `Result[Value, string]`: the note after replacing it whole. `version` is required |
| `vault::patch(id: string, fields: Value, version: string)` | `Result[Value, string]`: set some fields and keep the rest and the blocks. A `Value::Null` removes a field. `""` takes the current version |
| `vault::append_text(id: string, field: string, text: string, version: string)` | `Result[Value, string]`: append paragraphs to content field `field`, one per run of text between blank lines. Text is inline markup. `""` takes the current version |
| `vault::append_blocks(id: string, field: string, blocks: Value, version: string)` | `Result[Value, string]`: append one content block, or a list of them, in their JSON form (`{ kind, fields, blocks }`) |
| `vault::delete(id: string, version: string)` | `Result[Value, string]`: move the note to the trash |
| `vault::restore(id: string)` | `Result[Value, string]`: take a note out of the trash |
| `vault::archive(id: string)` | `Result[Value, string]`: archive the note as it is now. It stays readable and editable, but queries and searches leave it out unless they ask for archived notes |
| `vault::unarchive(id: string)` | `Result[Value, string]`: take the note out of the archive |
| `vault::daily(date: string)` | `Result[Value, string]`: the daily note for `YYYY-MM-DD`, created when there is none. `""` is today. The result has `created: bool` |

```rust
use vault

fn run() -> Result[(), string] {
    let day = vault::daily("")?
    let id = day.get("id").unwrap().as_string().unwrap()
    vault::append_text(id, "body", "Backup finished at " + fmt("{}", vault::now()), "")?
    vault::patch("01JB8Z4K2MQ7V3X9T5R6W8Y2ZB", Value::Map(#{ "done": Value::Bool(true) }), "")?
    Ok(())
}
```

`append_text` and `append_blocks` name the field as the type does. The daily type's content field is the one its type file defines; check it with `vault::types()`.

In a `before_*` handler, `create`, `update`, `patch`, `append_*` and `delete` join the write in progress. `restore`, `daily`, `upload_asset` and `add_term` are refused there; use an `after_*` handler. A migration cannot change other notes at all.

### Assets

| Function | Returns |
|----------|---------|
| `vault::assets()` | `Result[List[Value], string]`: every asset, as `{ id, filename, size }` |
| `vault::read_asset(id: string)` | `Result[Value, string]`: `{ id, filename, size, base64 }` |
| `vault::upload_asset(filename: string, base64: string)` | `Result[Value, string]`: store the bytes as a new asset |
| `vault::upload_asset_file(path: string)` | `Result[Value, string]`: store a file as a new asset, named after the file. The path must be one the script may read with `fs` |

An HTTP response carries its bytes as `body_base64`, so a downloaded image goes straight in:

```rust
use vault
use http

fn run() -> Result[string, string] {
    let r = http::get("https://images.example.com/today.png", #{})?
    let asset = vault::upload_asset("today.png", r.body_base64)?
    Ok(asset.get("id").unwrap().as_string().unwrap())
}
```

### Types and taxonomies

| Function | Returns |
|----------|---------|
| `vault::types()` | `Result[List[Value], string]`: every type, as `{ name, label, plural, description, fields }`. Each field is `{ name, label, kind, required, multiple, computed }`, plus `taxonomy` for a taxonomy field |
| `vault::taxonomies()` | `Result[List[Value], string]`: every taxonomy, as `{ name, label, description, allow_new, terms }`, where `terms` lists every term path |
| `vault::add_term(taxonomy: string, path: string)` | `Result[Value, string]`: add a term to an open taxonomy (`allow_new = true`). Only the last name in the path may be new. Returns `{ taxonomy, path, added }`; adding a term that exists already changes nothing |

### Notifying the user

| Function | Returns |
|----------|---------|
| `vault::notify(title: string, body: string, note: string)` | `Result[Value, string]`: add a notification under the bell in the web app, as `{ id, title, body, note, source }`. Pass `""` for no body or no note. Clicking it opens `note` |

The title is required and at most 200 characters, and the body at most 2000; the app shows the body's first two lines. A note that does not exist or is in the trash is an error. The notification's source is the run: `task:<name>` for a scheduled script and `handler:<type>.<event>` for a handler, such as `handler:invoice.after_create`. `notify` works in `after_*` handlers and scheduled scripts; a `before_*` handler or a migration gets an error, because it runs inside a write, which may be refused or be a dry run.

```rust
use vault

fn run() -> Result[(), string] {
    let page = vault::query_page("from task where !done && due < today()", 1, "")?
    let overdue = page.get("total").unwrap().as_int().unwrap()
    if overdue == 0 {
        return Ok(())
    }
    let day = vault::daily("")?
    let id = day.get("id").unwrap().as_string().unwrap()
    vault::notify(fmt("{} tasks are overdue", overdue), "See today's note.", id)?
    Ok(())
}
```

A scheduled script that fails and has no retry left notifies the user itself, with a link to the task's editor; it need not notify about its own failure.

### Commanding a device

| Function | Returns |
|----------|---------|
| `vault::device_command(device: string, name: string, payload: Value)` | `Result[Value, string]`: queue command `name` for a paired device, named by its id or its name, as `{ id, device, name, payload, status, result, source, created, expires, updated }` with `status` `"pending"` |

The command must be one the device's `devices/<platform>.wcl` declares, and one the user enabled for that device in Settings > Devices; anything else is an error and queues nothing. A device name two devices share is an error too: name the device by its id. `payload` is the command's payload from [Device commands](#device-commands), at most 64 KiB as JSON. The command's source is the run, as for `vault::notify`.

The device fetches the command, runs it, and reports `done` or `failed` with a result. A command the device has not finished within a day expires. Follow a command through `GET /api/devices/<id>/commands`. `device_command` works in `after_*` handlers and scheduled scripts; a `before_*` handler, a migration or a script running on a device gets an error.

```rust
use vault

fn after_create(note: Value) -> Result[(), string] {
    let title = note.get("note").unwrap().get("fields").unwrap().get("title").unwrap()
    let sms = Value::Map(#{
        "to": Value::String("+61400000000"),
        "body": Value::String("Invoice filed: " + title.as_string().unwrap()),
    })
    vault::device_command("Pixel", "send_sms", sms)?
    Ok(())
}
```

### Time and the log

| Function | Returns |
|----------|---------|
| `vault::now()` | `int`: the time now, in milliseconds since the Unix epoch |
| `vault::today()` | `string`: today as `YYYY-MM-DD`, in a scheduled script's `timezone` (UTC otherwise) |
| `vault::log(message: string)` | Adds a line to the run's log. `print` and `println` write there too |

A scheduled script's log is kept with each run, under Settings > Scheduled and in `GET /api/tasks/<name>/runs`.

## `http`

| Function | Returns |
|----------|---------|
| `http::get(url: string, headers: Map[string, string])` | `Result[Response, string]` |
| `http::post(url: string, headers: Map[string, string], body: string)` | `Result[Response, string]` |
| `http::request(method: string, url: string, headers: Map[string, string], body: string)` | `Result[Response, string]` |

`Response` is `{ status: int, headers: Map[string, string], body: string, body_base64: string }`. Header names are lower case. `body` replaces bytes that are not UTF-8; `body_base64` holds them exactly.

A call reaches only the hosts in the run's `allow_domains`. `["*"]` allows any http or https host, including hosts on your own network. Redirects are never followed. A status such as 404 is a response, not an error; check `status`.

```rust
use http
use json
use vault

fn run() -> Result[string, string] {
    let r = http::get("https://api.example.com/weather?city=Melbourne", #{ "accept": "application/json" })?
    if r.status != 200 {
        return Err(fmt("the weather API answered {}", r.status))
    }
    let data = json::parse(r.body)?
    let sky = data.get("sky").unwrap().as_string().unwrap()
    vault::append_text(vault::daily("")?.get("id").unwrap().as_string().unwrap(), "body", "Weather: " + sky, "")?
    Ok(sky)
}
```

## `secrets`

| Function | Returns |
|----------|---------|
| `secrets::get(name: string)` | `Result[string, string]`: the value of a secret the run names in `secrets`. Values are set in Settings > Secrets and never shown again |

## `fs`

Scheduled scripts only. A task's `read_paths` may be read, and its `write_paths` read and written.

| Function | Returns |
|----------|---------|
| `fs::read_text(path: string)` | `Result[string, string]`: a UTF-8 file |
| `fs::read_bytes(path: string)` | `Result[string, string]`: a file's bytes in base64 |
| `fs::write_text(path: string, text: string)` | `Result[(), string]`: create or replace a file. Its directory must exist |
| `fs::append_text(path: string, text: string)` | `Result[(), string]`: append to a file, creating it when missing |
| `fs::write_bytes(path: string, base64: string)` | `Result[(), string]`: create or replace a file from base64 bytes |
| `fs::list(path: string)` | `Result[List[Entry], string]`: a directory's entries by name. `Entry` is `{ name, path, is_dir, size, modified }`; `modified` is in milliseconds since the Unix epoch |
| `fs::exists(path: string)` | `Result[bool, string]` |
| `fs::mkdir(path: string)` | `Result[(), string]`: create a directory and any missing parents |
| `fs::remove(path: string)` | `Result[(), string]`: remove a file. Directories are refused |
| `fs::join(base: string, name: string)` | `string`: `base/name` |

Every path is checked before it is used:

- It must be absolute and must not contain `..`.
- Symbolic links are resolved, and the real path must lie inside an allowed directory, itself resolved. A link out of an allowed directory is refused, and so is writing through a link to nothing.
- Anything inside the vault folder is refused, for reading and writing alike, even when an allowed directory contains the vault. Change notes with `vault`, never by editing vault files. The vault's `.server` folder holds the password hash and tokens.
- A file is opened without following a link in its last part, so a link swapped in after the check is refused rather than followed.

```rust
use fs
use vault

fn run() -> Result[int, string] {
    let rows = vault::query("from task where done sort updated desc limit 100")?
    let out = fs::join("/home/me/exports", "done-" + vault::today() + ".csv")
    fs::write_text(out, "id,title\n")?
    for row in rows {
        let id = row.get("id").unwrap().as_string().unwrap()
        let title = row.get("title").unwrap().as_string().unwrap()
        fs::append_text(out, id + "," + title + "\n")?
    }
    Ok(rows.len())
}
```

## Device events

A device runs the handlers its platform's `devices/<platform>.wcl` names for each event, in file order (`docs/vault-format.md` "Device files"). A handler function takes one `Value`:

```text
{ event, device, at, data }
```

| Member | Meaning |
|--------|---------|
| `event` | The event name, such as `"sms_received"` |
| `device` | The id of the device the event happened on |
| `at` | When the device captured the event, in milliseconds since the Unix epoch |
| `data` | The event's payload, below |

What a handler returns is ignored, except that `Err(message)` records the run as failed. Before a handler runs, the device drops events from anyone on its exclusion list and SMS whose body matches its skip pattern (Settings > Devices).

These conventions hold in every payload:

- A phone number is in two members: `number`, normalised to E.164 with the phone's region, such as `"+61400000000"`, and `number_raw`, as the phone holds it. `number` is `null` when the raw value is not a phone number, such as a short code or a sender name.
- A time is a string in the form a `datetime` field takes, RFC 3339 with an offset, such as `"2026-10-20T09:30:00+11:00"`.
- `contact` is the phone contact the number belongs to, as `{ id, name }`, or `null` when no contact has it. `id` is the phone's contact id.
- `device_ref` identifies the item in the phone's own store, such as `"sms:4412"`, `"mms:87"`, `"call:1203"`, or a notification key for RCS. A handler de-duplicates on it: the device can deliver an event again after a restart or a catch-up.
- `thread_ref` identifies the conversation in the phone's own store.
- An attachment is `{ asset, filename, mime, size }`, where `asset` is the id of the asset the device stored it as in the vault.

| Event | `data` |
|-------|--------|
| `sms_received` | `{ device_ref, thread_ref, number, number_raw, contact, body, sent }`. `sent` is when the sender sent it |
| `sms_sent` | `{ device_ref, thread_ref, number, number_raw, contact, body, sent }`. `number` is the recipient |
| `mms_received` | `{ device_ref, thread_ref, number, number_raw, contact, to, subject, body, attachments, sent }`. `number` is the sender. `to` lists every recipient of a group message, other than this phone, as `{ number, number_raw, contact }`. `subject` and `body` are `""` when the message has none |
| `rcs_received` | `{ device_ref, thread_ref, sender, conversation, body, sent, partial }`. Read from a Google Messages notification: `sender` is the display name it shows, `conversation` the conversation title or `null` for a one-to-one chat, and `thread_ref` the notification's conversation key. There is no number. `partial` is always `true` |
| `call_ended` | `{ device_ref, direction, number, number_raw, contact, started, duration }`. `direction` is `"in"`, `"out"` or `"missed"`; `duration` is in whole seconds, `0` for a missed call |
| `contact_changed` | `{ contact_id, name, phones, addresses, updated }`. `phones` lists every number as E.164, the ones that cannot be normalised left out; `addresses` lists every email address. `updated` is when the phone last changed the contact |
| `contact_deleted` | `{ contact_id }` |
| `share_received` | `{ text, subject, url, attachments }`. What another app shared: `text` and `subject` are `""` when it sent none, `url` is the first link in `text` or `null`, and `attachments` lists the shared files |

## Device commands

A command runs on the device through the script function its `command` block names (`docs/vault-format.md` "Device files"). The function takes one `Value`:

```text
{ id, command, payload }
```

`id` is the command's id and `payload` what `vault::device_command` queued. `Ok(value)` reports the command `done` with `value` as its result; `Err(message)` reports it `failed` with `message` as its result.

| Command | `payload` | Result when `done` |
|---------|-----------|--------------------|
| `send_sms` | `{ to, body }`. `to` is a number, E.164 or as dialled | `{ device_ref }` of the sent message. Needs the device's SMS-send switch |
| `upsert_contact` | `{ contact_id, name, phones, addresses, person }`. `contact_id` is `null` to add a contact. `person` is the id of the `person` note the contact mirrors | `{ contact_id }`. Needs the device's contacts-write switch |
| `delete_contact` | `{ contact_id }` | `{}`. Needs the device's contacts-write switch |
| `show_notification` | `{ title, body, note }`. `body` and `note` are `""` for none; tapping the notification opens `note` | `{}` |

A device reports on a command at `POST /api/devices/<id>/commands/<cid>/result` with `{ "status": "delivered" }` once it has the command, then `{ "status": "done" | "failed", "result": ... }`. A command that is finished or expired answers 409.
