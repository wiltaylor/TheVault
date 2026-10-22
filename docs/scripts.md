# Scripts reference

Scripts are wscript files, `scripts/<name>.ws`. They run as type handlers (`docs/vault-format.md` "Handlers"), schema-change migrations, scheduled scripts (`docs/vault-format.md` "Task files", Settings > Scheduled in the web client), and on a paired phone as device event handlers and device commands (`docs/vault-format.md` "Device files"). This page lists every host function a script can call, and the payload of every device event and command.

## What each kind of run can use

| Module | `before_*` handlers, migrations | `after_*` handlers | Scheduled scripts | Device handlers and commands |
|--------|---------------------------------|--------------------|-------------------|------------------------------|
| `vault` | yes | yes | yes | yes |
| `http` | no | yes | yes | no |
| `secrets` | no | yes | yes | no |
| `fs` | no | no | yes | no |
| `device` | no | no | no | yes |
| `value`, `json`, `math`, `regex` | yes | yes | yes | yes |

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

A block's JSON form is the block of `docs/vault-format.md` "Content blocks": `kind`, its items under `fields`, and the blocks inside it under `blocks`. A toggle holds the blocks it opens to show, and an admonition (a callout) holds its own:

```rust
use vault

fn run() -> Result[(), string] {
    let id = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB"
    let folded = Value::Map(#{
        "kind": Value::String("toggle"),
        "fields": Value::Map(#{ "level": Value::Int(2), "text": Value::String("Details") }),
        "blocks": Value::List([Value::Map(#{
            "kind": Value::String("paragraph"),
            "fields": Value::Map(#{ "text": Value::String("Shown when opened") }),
        })]),
    })
    let callout = Value::Map(#{
        "kind": Value::String("admonition"),
        "fields": Value::Map(#{ "kind": Value::String("warning"), "title": Value::String("Backup failed") }),
        "blocks": Value::List([Value::Map(#{
            "kind": Value::String("paragraph"),
            "fields": Value::Map(#{ "text": Value::String("Check the disk.") }),
        })]),
    })
    vault::append_blocks(id, "body", Value::List([folded, callout]), "")?
    Ok(())
}
```

An admonition's `kind` is one of `note`, `info`, `tip`, `success`, `warning`, `danger`, `question` and `quote`; `title`, `colour`, `icon` and `collapsible` are optional. A wrong kind, a colour outside the palette or an unknown icon name fails the write with the reason.

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
| `secrets::get(name: string)` | `Result[string, string]`: the value of a secret the run names in `secrets`. Values are set in Settings > Secrets, or by `secrets::set`, and never shown again |
| `secrets::set(name: string, value: string)` | `Result[(), string]`: store `value` as secret `name`, encrypted like `PUT /api/secrets/<name>`, replacing what it held. Refused for a name the run's `secrets` list does not name, and for a value over 64 KiB |

A script reads and writes only the secrets its run names, and cannot list them. `secrets::set` writes no note and no note history, so it suits state a scheduled script keeps between runs, such as a sync cursor. A `before_*` handler or a migration has no `secrets` module, so it can neither read nor write one.

### The Google sync's secrets

The Google sync keeps its settings and state in secrets, never in notes (google-sync D21). Each is a JSON object string, except `google_account`:

| Secret | Written by | Read by | Holds |
|--------|------------|---------|-------|
| `google_client_id`, `google_client_secret`, `google_refresh_token` | Settings > Integrations | every Google script | The OAuth client and the sign-in |
| `google_account` | Settings > Integrations, on connect | the Integrations page | The connected address, as plain text |
| `google_settings` | Settings > Integrations | every Google script | The settings, below |
| `google_gmail_state` | the `google_gmail` task | the `google_gmail` task | `{ history_id, listed, backfill_page, last_run, last_error, notified_invalid_grant }` |
| `google_calendar_state` | the `google_calendar` task | the `google_calendar` task | `{ sync_tokens, window_end, last_run, last_error, notified_invalid_grant }` |

`google_settings` is `{ gmail_mode, gmail_labels, gmail_window_days, gmail_backfill_to, calendars, default_calendar, endpoints }`:

| Key | Holds | When missing |
|-----|-------|--------------|
| `gmail_mode` | `labels` pulls the labels in `gmail_labels`; `all` pulls all mail | `labels` |
| `gmail_labels` | Gmail label names or ids, matched without case | `[]` |
| `gmail_window_days` | How many days back the first pull reaches, 1 to 3650 | `90` |
| `gmail_backfill_to` | A date, `YYYY-MM-DD`, older mail is paged back to, a page each run; or null | null |
| `calendars` | The ids of the calendars to pull, such as `primary` | `["primary"]` |
| `default_calendar` | The calendar a new event note goes to when it names none | `primary` |
| `endpoints` | The test seam: `{ token, gmail, calendar, userinfo }` URLs. The Integrations page never shows or sets it | Google's own hosts |

A missing `google_settings` means every default. In the two state objects, `last_run` is an RFC 3339 time, `last_error` the last run's error or null, and `notified_invalid_grant` is true once the run has told the user Google refused the sign-in; it goes back to false when a token refresh succeeds. `history_id` and `backfill_page` are Gmail's history id and back-fill page token. `listed` is what the last finished window listing covered, `{ mode, labels, window_days }` with `labels` the sorted Gmail label ids, or null before one: a run whose settings differ lists the window again besides reading history (all of it for a new mode or window, only the added labels otherwise, nothing when labels were only removed) and starts the back-fill over, and a run with nothing to list saves no `history_id`. `sync_tokens` holds each calendar's sync token by calendar id, and `window_end` is the date recurring events have occurrence notes up to. Disconnecting Google sets the cursors (`history_id`, `listed`, `backfill_page`, `sync_tokens`, `window_end`) back to null or empty.

Each task writes only its own state secret, so the two never overwrite each other, and its task file's `secrets` list names exactly what it reads and writes. The write-back handler reads `google_settings` and writes nothing but notes.

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

## `device`

Device event handlers and device commands only. They run on the paired phone, and their `vault` calls change the phone's copy of the vault, which syncs with the home server like any other change. A device script's changes are recorded as made by `handler <script>`.

| Function | Returns |
|----------|---------|
| `device::id()` | `string`: this device's id |
| `device::sms_list(filter: Value)` | `Result[List[Value], string]`: SMS, newest first, in the form of the `sms_received` payload. `filter` may hold `thread_ref`, `number`, `since` (a time) and `limit` |
| `device::sms_get(device_ref: string)` | `Result[Value, string]`: one SMS |
| `device::calls_list(filter: Value)` | `Result[List[Value], string]`: calls, newest first, in the form of the `call_ended` payload. `filter` may hold `number`, `since` and `limit` |
| `device::contacts_list(filter: Value)` | `Result[List[Value], string]`: contacts, in the form of the `contact_changed` payload. `filter` may hold `name` (the exact display name), `number` (E.164) and `limit` |
| `device::contacts_get(contact_id: string)` | `Result[Value, string]`: one contact |
| `device::contacts_upsert(contact: Value)` | `Result[Value, string]`: add a contact, or change it when `contact_id` is not null, from `{ contact_id, person }` with any of the contact's detail keys, as in the `upsert_contact` payload. A detail key left out leaves that detail of the contact alone. Returns `{ contact_id }`. Needs the device's contacts-write switch |
| `device::contacts_delete(contact_id: string)` | `Result[Value, string]`: delete a contact. Needs the device's contacts-write switch |
| `device::sms_send(to: string, body: string)` | `Result[Value, string]`: send an SMS. Returns `{ device_ref }`. Needs the device's SMS-send switch |
| `device::show_notification(title: string, body: string, note: string)` | `Result[Value, string]`: show a phone notification that opens `note` when tapped. Pass `""` for no body or no note |

A read is an error when Android has not given the app the permission for it. A write or a send with its switch off in Settings > Devices is an error, whatever the script, and so is `sms_send` without Android's SEND_SMS permission. Deleting a `message` note never deletes the SMS on the phone.

On the home server, a script a device file names is checked as a device script and never run there.

```rust
use vault
use device

fn sms_received(event: Value) -> Result[(), string] {
    let data = event.get("data").unwrap()
    let body = data.get("body").unwrap().as_string().unwrap()
    if body.contains("urgent") {
        device::show_notification("Urgent SMS", body, "")?;
    }
    Ok(())
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

The device stores each event in a queue in its `state.db` before any handler runs, so an event survives the app being stopped. A failed run leaves the event queued and tries it again on the next pass, up to three passes. The handlers of one event run in file order, and the first that fails stops the rest, so a handler should check `device_ref` before it creates a note.

An event whose handler fails on all three passes is marked failed and kept. The Android app then shows one notification, "A message could not be saved", however many fail together, and its next sync round reports each failed event to the home server, which adds one notification under the bell with source `device:<device id>`. Settings > Devices > This device in the app lists every failed and skipped event, newest first, under "Capture problems": the event, when it happened, the contact or number it names, the handler's error or why it was skipped, and the passes it took. The count of failed events is a badge on This device. Retry gives a failed event three more passes and runs the first at once; Dismiss hides it.

The app reports a failed event at `POST /api/devices/<id>/event-failures` with its device token, which may report only for its own device; an admin token may report for any. The body is `{ "event": ..., "who": ..., "message": ..., "attempts": ... }`, where `event` is a device event name, `who` the contact, sender or number it names (`null` for none), `message` the handler's error and `attempts` the passes it took; other members are ignored. The server answers 201 with the notification it added: titled by what was lost (`A message could not be saved`, `A call could not be saved`, `A contact change could not be saved` or `A share could not be saved`), with a body naming the event, the device and the error, and opening `/settings/devices#this-device`. An event name that is not a device event answers 400, another device's token 403, and an unknown device 404. The app reports each failure once, and stops reporting one the server refuses with a 4xx. The phone does not mirror these notifications into its own shade, since it already shows its own.

The exclusion list matches:

| Entry | Drops |
|-------|-------|
| `number` | Any event whose `number` or `number_raw`, or a group MMS recipient's, is the number, ignoring spaces and punctuation; and a contact event for a contact with the number |
| `contact` | Any event whose `contact` has that `id` or that name (ignoring case), an RCS message whose `sender` is the name, and a contact event for that contact |
| `thread` | Any event whose `thread_ref` is the value |

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
| `mms_received` | `{ device_ref, thread_ref, number, number_raw, contact, to, subject, body, attachments, sent }`. `number` is the sender. `to` lists every recipient of a group message, other than this phone, as `{ number, number_raw, contact }`. `subject` and `body` are `""` when the message has none. `attachments` lists every part other than text and layout, each `{ asset, filename, mime, size }`, stored as an asset of the local vault and named with the extension of its media type, as for `share_received` |
| `rcs_received` | `{ device_ref, thread_ref, sender, conversation, body, sent, partial }`. Read from a Google Messages notification: `sender` is the display name it shows, `conversation` the conversation title or `null` for a one-to-one chat, and `thread_ref` the notification's conversation key. There is no number. `partial` is always `true` |
| `call_ended` | `{ device_ref, direction, number, number_raw, contact, started, duration }`. `direction` is `"in"`, `"out"` or `"missed"`; `duration` is in whole seconds, `0` for a missed call |
| `contact_changed` | `{ contact_id, name, phones, phone_labels, emails, email_labels, addresses, address_labels, organisation, job_title, nickname, birthday, websites, contact_note, photo, updated, person, synced }`. `phones` lists every number as E.164, the ones that cannot be normalised left out; `emails` every email address; `addresses` every postal address, each the phone's formatted address with its line breaks joined by `", "`. Each `*_labels` list holds one label per value, in the same order: `"mobile"`, `"home"`, `"work"`, `"main"`, `"other"`, or the phone's custom label text. `organisation`, `job_title`, `nickname` and `contact_note` (the contact's note) are strings, `""` for none; `birthday` is `"YYYY-MM-DD"`, or `"--MM-DD"` when the phone has no year, `""` for none; `websites` lists the contact's websites as the phone holds them. `photo` is `{ asset, hash }` or `null`: the app stores the contact photo as an asset of the local vault before queuing the event, as for MMS attachments, and `hash` is the SHA-256 hex of its bytes. `updated` is when the phone last changed the contact. `person` is the id of the `person` note the contact is linked to, and `synced` is what the contact and the person last agreed on, with the detail keys the last `upsert_contact` carried (`photo` as `{ asset, hash }` of the photo written to the phone); both are `null` on a contact that was never linked. Links are per vault: both are also `null` when the contact's link belongs to another vault than the one the local vault is a copy of, so the handler matches or creates a person in this vault as for a contact never linked (see "Contact links are per vault"). A detail key missing from `data` was not reported, as by an app from before it existed, and a key missing from `synced` has no agreed value yet. An app from before `emails` had their own key reports email addresses as `addresses`: an event without `emails` is read that way |
| `contact_deleted` | `{ contact_id }` |
| `share_received` | `{ text, subject, url, attachments }`. What another app shared: `text` and `subject` are `""` when it sent none, `url` is the first link in `text` or `null`, and `attachments` lists the shared files, each `{ asset, filename, mime, size }`: the app stores every shared file as an asset of the local vault before the event is queued. A stored file whose name lacks the extension of its media type gets one, so `image000001` sent as `image/jpeg` is stored as `image000001.jpg` and shows as an image |

On Android the app captures these events itself, with no window open (D7, D17):

- An SMS when Android announces it, an MMS when it appears in the phone's message store, a call when the phone goes back to idle, and a contact edit when the contacts store changes. The app reads each from the phone's own store, so `device_ref` is the store's row: `sms:<id>`, `mms:<id>` or `call:<id>`. `thread_ref` is the store's thread id, shared by the SMS and MMS of one conversation.
- Every hour, a catch-up reads each store from where capture last stopped, so nothing is missed while the app is stopped, and nothing is captured twice.
- Capture starts from the newest items when the app is set up. Earlier messages, calls and contact changes arrive only through an import: the user turns it on in the capture setup, which is off by default, and picks a range of dates, or starts one at any time with "Import from phone" (see below).
- An import reads the phone's SMS, MMS, call log and contacts again over a range of dates, whatever capture has already read, and runs each item as the event capture would have made of it, so the handlers decide the notes. Before an SMS, MMS or call is queued, the local vault is asked for a `message` or `call` note with its `device_ref`, archived notes included; one that has a note is left alone. So an import never duplicates a note, and a message whose note was deleted, or one capture missed, gets exactly one note again. Each contact changed within the range runs as a `contact_changed` event. RCS messages cannot be imported: Google Messages keeps them to itself, and the app only sees them as their notifications arrive.
- "Import from phone" is in Settings > Devices: in the Android app under This device, for the phone the app runs on, and on an Android phone's card, where it queues the `import_history` command for that phone. The command palette's "Import messages and calls" opens it in the app. This device also has "Check for new messages and calls now", which runs the hourly catch-up at once. Either action says which Android permission is missing, if any, and offers the permission step.
- Only received MMS are captured, since no event carries a sent one. A group MMS's `to` leaves out this phone's own numbers when Android tells the app what they are.
- A `person` linked to a phone contact that changes anywhere, once the phone syncs the change, queues a `contact_changed` event for the contact as the phone holds it, so the same handler carries the change to the phone. This happens only while the device's contacts-write switch is on.
- A `person` with a phone number, an email address or a postal address that no contact on a device mirrors yet reaches that device as an `upsert_contact` command, which the home server queues itself when the person is created, or when any detail a contact holds changes. It queues one only for a device whose contacts-write switch is on and that has `upsert_contact` enabled, and never for the device that made the change. When the device reports the command done, the server links the person to the contact in the result, as that device, in case the person had not synced to the device when the command ran. Deleting a person queues nothing: the phone keeps its contact (D16).
- Something shared to TheVault from another app is a `share_received` event, queued and run at once, with or without a connection and before pairing too, since it needs only the local vault. One share makes what its handler makes, by default one fleeting note in the inbox. A handler that fails leaves the event queued for the next pass, like any device event. When the device file declares no `share_received` handler, the app writes the fleeting note the default `device_share` script would, so a share is never silently lost.

The default `device_contacts` script keeps contacts and `person` notes in step field by field (D16). The fields are the name; `phones`, `emails` and `addresses`, each with its label list as one field, so a label moves with its value and a changed label alone is a change; `organisation`, `job_title`, `nickname`, `birthday`, `websites` and `contact_note`; and `photo`. A contact detail goes to the `person` field of the same name, the name to `title`. For each field the event reports:

- A field changed on one side since `synced` takes that side; a field changed on both sides takes the side changed most recently.
- With no agreed value in `synced`, a side that lacks the field gives way to the side that has it; otherwise the side changed most recently wins. Sides that already agree count as agreed.
- A field the event does not report keeps the person's value.
- The photo compares by hash: the contact side by its `hash` against `synced.photo.hash`, the person side by its asset id against `synced.photo.asset`. The same photo stored again on the phone as a new asset is no change, and a person that takes the phone's photo takes its asset.
- A website without a scheme gets `https://`, and one that is still not a URL is dropped.
- A label list the event lacks gives each value the person's label for it. A label list written to the person or the contact is cut or padded with `""` to one label per value.
- A field that settles empty is removed from the person.

When the contact lacks a settled value, or a field has no agreed value that both sides hold, the script writes every detail to the contact with the person's id, which needs the contacts-write switch; the phone records them as `synced`. To an app from before `emails` had their own key, it also sends the email addresses as `addresses`. A new contact makes a person with every detail it reports. Deleting the contact on the phone unlinks the person and keeps it, and a contact whose person was deleted in the vault is left alone rather than making the person again.

### Contact links are per vault

The Android app keeps a contact's link in a data row of its own in the phone's contacts database (`vnd.android.cursor.item/vnd.dev.wiltaylor.thevault.person`): `data1` is the person's id, `data2` what the two sides last agreed on, `data3` the format of `data2`, and `data4` the id of the vault the person is in (the home server's `vault_id`, `docs/sync.md`). The row lives in the contacts database, not in the app, so it outlives the app being uninstalled, its data cleared, and the phone joining another vault (ticket 143).

- A row naming the vault the local vault is a copy of is the contact's link.
- A row naming another vault is ignored: the contact reads as never linked, so `device_contacts` matches it to a person of this vault by number or makes a new one, and nothing is based on the other vault's people.
- A row from an app before ticket 143 names no vault. It counts as this vault's when the local vault holds its person, and is ignored otherwise.
- When a contact is linked (an `upsert_contact` or the handler's write back, which need the contacts-write switch), its row is written again for this vault. With the switch off, a stale row stays on the phone and is ignored.
- Once after joining a vault, after its first successful round and with the contacts-write switch on, the app removes every row naming another vault, and every older row whose person the local vault does not have, and marks an older row whose person it has as this vault's.

## Device commands

A command runs on the device through the script function its `command` block names (`docs/vault-format.md` "Device files"). The function takes one `Value`:

```text
{ id, command, payload }
```

`id` is the command's id and `payload` what `vault::device_command` queued. `Ok(value)` reports the command `done` with `value` as its result; `Err(message)` reports it `failed` with `message` as its result.

| Command | `payload` | Result when `done` |
|---------|-----------|--------------------|
| `send_sms` | `{ to, body }`. `to` is a number, E.164 or as dialled | `{ device_ref }` of the sent message. Needs the device's SMS-send switch |
| `upsert_contact` | `{ contact_id, name, phones, phone_labels, emails, email_labels, addresses, address_labels, organisation, job_title, nickname, birthday, websites, contact_note, photo, person }`: the detail keys of the `contact_changed` event, with `photo` an asset id or `null` for no photo. Each label list has one label per value, `""` where the person has none. `contact_id` is `null` to add a contact, or to change the contact that already holds `person`. `person` is the id of the `person` note the contact mirrors; the contact stores it, and the detail keys the payload carries as what the two last agreed on. A detail key left out leaves that detail of the contact alone. The home server queues one, with `source` `contacts`, for a person no contact on the device mirrors yet | `{ contact_id }`. Needs the device's contacts-write switch |
| `delete_contact` | `{ contact_id }` | `{}`. Needs the device's contacts-write switch |
| `show_notification` | `{ title, body, note }`. `body` and `note` are `""` for none; tapping the notification opens `note` | `{}` |
| `import_history` | `{ kinds, since, until }`. `kinds` lists some of `sms`, `mms`, `calls` and `contacts`, all of them when left out; `rcs` is refused. `since` and `until` are RFC 3339 times or milliseconds, `null` for no bound. The function answers the import to run, by default the payload, and the app then runs it as described under Android capture | `{ kinds: [{ kind, found, imported, notes, existing, skipped, failed, missing_permission, error, done }], since, until, done, message }`: per kind, the items found in the range, those whose events ran, the notes they made, those that already had a note, those excluded (D25), and those whose handler failed. `failed` with the same report when no kind could be read, for example for want of a permission |

A device reports on a command at `POST /api/devices/<id>/commands/<cid>/result` with `{ "status": "delivered" }` once it has the command, then `{ "status": "done" | "failed", "result": ... }`. A command that is finished or expired answers 409.

Settings > Devices queues a command itself at `POST /api/devices/<id>/commands` with `{ "name": ..., "payload": ... }`, under the same rules as `vault::device_command`. An admin may queue any command. A device's own `write` token, which is what an app's Settings > Devices reaches the home server with, may queue only `import_history`, and only for itself. A long command reports `{ "status": "delivered", "result": { "progress": ... } }` while it runs, at most every two seconds; `import_history` reports its report that way, so the page shows how far it got.

The Android app fetches its pending commands when a `device.command` push arrives, when the app opens, and, on a phone with no UnifiedPush distributor, every 15 minutes. It first takes its settings from the home server, so a command enabled a moment ago runs. It reports each command `delivered` before running it, so a run that a crash cuts short is not repeated: the command expires instead. `send_sms` also needs Android's SEND_SMS permission; without it the command fails with a message that says so.

## The Google Calendar sync

The shipped task `google_calendar` runs `scripts/google_calendar.ws` every five minutes once it is unpaused (google-sync D12, D19). It reads `google_settings` and its own `google_calendar_state` (see [The Google sync's secrets](#the-google-syncs-secrets)), and ends successfully with a log line when there is no `google_refresh_token`: Google is not connected. It exchanges the refresh token for an access token at the settings' `endpoints.token` (Google's own token endpoint when the key is absent), and calls the Calendar API at `endpoints.calendar`, by default `https://www.googleapis.com/calendar/v3`. The run's log ends with what it did, such as `2 created, 1 updated, 0 trashed`.

First the run reads the user's calendar list, `calendarList.list` at `<endpoints.calendar>/users/me/calendarList`, every page of it, for each calendar's name: its `summaryOverride`, the name the user gave it, else its `summary`. The calendar Google marks `primary` is also named under the id `primary`. A list that fails is logged, and the run goes on without names.

For each calendar id in the settings' `calendars` (`["primary"]` when empty):

- **Listing.** `events.list` with `singleEvents=false` and the calendar's sync token, or with none on the first run. A 410 drops the token and lists the calendar in full. A full listing names every event there is, so a note of that calendar whose event or series it leaves out goes to the trash. The next sync token is stored in the state's `sync_tokens` under the calendar id.
- **Single events** become one `event` note each, found again by `google_ref`, the event id.
- **Series.** A series master that is new or changed has its occurrences fetched with `events.instances` from 30 days before today to the state's `window_end` (a year after today), each occurrence one note whose `google_ref` is the occurrence id, `<series id>_<original start>`, and whose `series` is the master's `recurrence` lines joined by a line break. An occurrence note in that span that Google no longer lists goes to the trash. The master itself has no note. A changed occurrence arrives on its own and updates its note.
- **Deletes.** A cancelled event or occurrence moves its note to the trash; a cancelled series moves every note of its occurrences there (D9).
- **Window roll.** When today has moved on since `window_end` was written, every series with an occurrence note on the calendar has the new days' occurrences fetched, from the old window end to the new one.
- `event.calendar` holds the calendar id as `calendars` lists it, such as `primary`, and `event.calendar_name` the calendar's name from the calendar list, such as `Personal` (ticket 103). A note whose name changed is updated, even when Google did not change its event. Without a name, as when the list failed, the note keeps the name it has. `calendar_name` is not synced back and is not in the `snapshot`.

Afterwards the run writes `window_end`, `last_run` and the tokens to its state, and sets `last_error` to null. A failure writes the error to `last_error` and fails the run. An `invalid_grant` answer from the token endpoint writes `last_error` as `invalid_grant: <Google's description>`, adds the notification "Google sign-in expired" unless `notified_invalid_grant` is already true, sets it, and ends the run successfully, so the task-failure notification does not fire as well (D18). No note is written for any of it.

### Snapshot and etag

Every event note the sync writes holds, in hidden fields, what Google last sent, so a change can be told apart by side (D10):

| Field | Holds |
|-------|-------|
| `google_ref` | The event id, or the occurrence id `<series id>_<original start>` |
| `etag` | Google's `etag` exactly as sent, quotes included, such as `"3181161784712000"`. Send it as `If-Match` |
| `snapshot` | A dict of strings, every key always present, `""` for a value Google did not send |

The `snapshot` keys hold each synced field in the form the note holds it:

| Key | From Google | Note field |
|-----|-------------|------------|
| `title` | `summary` | `title` |
| `start`, `end` | `start.date` and the day before `end.date` for an all-day event (Google's end date is exclusive, the vault's range ends on its last day); `start.dateTime` and `end.dateTime` as sent, with their offsets, for a timed one | `when.start`, `when.end` |
| `location` | `location` | `location` |
| `description` | `description` with carriage returns and line-ending spaces removed and blank lines dropped | `description`: one paragraph per line, the lines joined with a line break to compare |
| `attendees` | Each `attendees[].email`, in Google's order, joined with a line break | `attendees_raw`, joined the same way. `attendees` links each address to the person whose `emails` hold it, ignoring case (D5) |
| `organiser` | `organizer.email` | `organiser` |
| `status` | `status`, `confirmed` when absent | `status` |
| `meet_url` | `hangoutLink` | `meet_url` |

A note's synced fields equal its `snapshot` in every key exactly when the vault has not changed them since the last sync. Applying a Google change to a note merges each field against the snapshot: a field changed only in Google takes Google's value, a field changed only in the vault keeps the vault's, and a field changed on both takes the side changed most recently, the note's `updated` against Google's `updated`. `start` and `end` merge as one field. The sync then writes Google's values as the new `snapshot` and Google's `etag`, so a field the vault kept still differs from the snapshot afterwards. A note Google did not change is not written.

## The Google Calendar write-back

The shipped `event` type runs `scripts/google_event.ws` after every create, update and delete of an event note, from the handler queue (google-sync D9, D10, D20). Its handlers name the three credential secrets and `google_settings`. It reads the settings' `endpoints` and `default_calendar` as the sync does, and refreshes an access token for each run that has something to send. A run with no `google_refresh_token` sends nothing and succeeds. An `invalid_grant` sends nothing and does not fail the run; the `google_calendar` task records it and notifies the user. The handler writes no secret.

A handler cannot tell who wrote a note, so the snapshot is the loop guard. A note whose synced fields equal its `snapshot` has no change of the vault's, and the run sends nothing, not even a token refresh. Every note the sync writes is in that state unless the vault kept one of its own changes in a merge, which the next handler run then sends. Times compare as instants, so `10:00:00Z` and `21:00:00+11:00` are the same start.

| Event | Sends |
|-------|-------|
| `after_update` | `PATCH` of the event, or of the one occurrence its `google_ref` names (D8), with `If-Match: <etag>` and only the field groups that differ from the snapshot. A 412 is answered by a `GET`, a field-by-field merge as the sync's (D10), and one more `PATCH` with Google's new etag of what still differs from Google; a second 412 fails the run. A 404 or 410 sends nothing more: the next sync trashes the note |
| `after_create` | With no `google_ref`: a `POST` into the note's `calendar`, else `default_calendar`, else `primary`. A note with no `when` yet is not sent; the first update that gives it one inserts it. With a `google_ref`: nothing for a note the sync just made (its `created` equals its `updated` and it equals its snapshot). Any other, such as a note restored from the trash, is looked up with a `GET`; when Google answers 404 or 410 or the event is cancelled, it is inserted as a new event (D20) and loses `series`, since it is no longer an occurrence of a series |
| `after_delete` | `DELETE` of the event or occurrence. A 404 or 410, as for a note the sync trashed because Google deleted the event, counts as done |

After a `POST` or `PATCH`, the handler writes Google's answer back to the note with `vault::update`: `etag`, the `snapshot` of what Google now holds, and, for an insert, `google_ref` and `calendar`. Each field group the note still holds as it was sent takes Google's value, so the note equals its snapshot and the `after_update` that write sets off sends nothing. A group the user changed again meanwhile keeps the user's value, for that next run to send.

The note fields map to the event as the sync's table above, inverted:

| Note | Sent as |
|------|---------|
| `title` | `summary` |
| `when` of dates | `start.date`, and `end.date` the day after the range's last day; `dateTime` set to null |
| `when` of datetimes | `start.dateTime` and `end.dateTime` as the note holds them; `date` set to null |
| `location` | `location` |
| `description` | Its paragraphs joined with a line break |
| `attendees_raw` | `attendees`, one entry per address, in order. Before a `PATCH` that changes them, the handler reads the event with a `GET` (or uses the one a 412 read), and an address Google already lists, compared ignoring case, keeps Google's whole entry: its `responseStatus`, `displayName`, and `optional` and `organizer` flags. A new address is sent as `{ email }`, and an address the note dropped is left out. An insert sends `{ email }` for each |
| `status` | `status` |

`organiser` and `meet_url` are Google's own and are never sent; the write-back takes Google's values for them.
