# Vault file format

This document defines the WCL vocabulary of every file in a vault: notes, types with their templates and handlers, taxonomies, scheduled tasks, device files, and the content, ink and canvas blocks that notes and templates contain. It settles OQ-S1 and overrides the illustrative note example in `docs/plans/thevault-prd/SPEC.md`.

The `store`, `schema`, `writer`, `index`, `query`, `script`, `scheduler`, `api`, `cli` and `web` components all read and write this format, and S3 expected files use it. When this document and the code disagree, the code is wrong.

Everything here sits under these decisions: D3 (field kinds), D4 and D5 (references), D7 (taxonomies), D14 (templates), D15 (canvas), D20 (data-only WCL), D21 (canonical format), D22 (layout), D23 (value encodings), D34 (content blocks), D41 (inline markup).

## Contents

- [Files at a glance](#files-at-a-glance)
- [Names and identifiers](#names-and-identifiers)
- [The data-only subset](#the-data-only-subset)
- [Value encodings](#value-encodings)
- [Type files](#type-files)
- [Field kinds](#field-kinds)
- [Templates](#templates)
- [Handlers](#handlers)
- [Note files](#note-files)
- [Content blocks](#content-blocks)
- [Inline markup](#inline-markup)
- [Query views](#query-views)
- [Ink](#ink)
- [Canvas](#canvas)
- [Taxonomy files](#taxonomy-files)
- [Task files](#task-files)
- [Device files](#device-files)
- [Shipped defaults and upgrades](#shipped-defaults-and-upgrades)
- [Validating vault files](#validating-vault-files)

## Files at a glance

| File kind | Path in the vault | Structural schema | Example |
|-----------|-------------------|-------------------|---------|
| Note | `notes/<type>/<ULID>.wcl` | `schema/common.wcl` plus a generated note schema | `vault/notes/specimen/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA.wcl` |
| Type | `types/<type>.wcl` | `schema/common.wcl` plus `schema/type.wcl` | `vault/types/specimen.wcl` |
| Taxonomy | `taxonomies/<taxonomy>.wcl` | `schema/taxonomy.wcl` | `vault/taxonomies/status.wcl` |
| Task | `tasks/<task>.wcl` | `schema/task.wcl` | `vault/tasks/daily_digest.wcl` |
| Device file | `devices/<platform>.wcl` | `schema/device.wcl` | `vault/devices/android.wcl` |
| Canvas | A `canvas` content block, or a `canvas` field value in a note | `schema/common.wcl` | `vault/notes/board/01JB8Z4K2MQ7V3X9T5R6W8Y2ZF.wcl` |

Paths in the last two columns are relative to `docs/vault-format/`. Scripts (`scripts/*.ws`) are wscript, not WCL, and this document does not define them. Assets (`assets/<ULID>/<filename>`) are stored byte for byte. Deleting an asset moves its file to the server folder, at `.vault/asset-trash/<ULID>/<filename>`, with the time of the delete in milliseconds since the Unix epoch in `.vault/asset-trash/<ULID>.deleted`. It can be restored from there for 30 days, after which it is purged. The asset trash is not vault data: a note never names it, and a rebuild ignores it.

The example vault under `docs/vault-format/vault/` is self-consistent: its notes use its types, and its types use its taxonomies. Notes that the examples reference but do not need (the parent, child and peer specimens) are left out. The scripts named by the example handlers, task and device file are left out too.

## Names and identifiers

| Name | Rule | Where it comes from |
|------|------|---------------------|
| Type name | `[a-z][a-z0-9_]*`, at most 64 characters | The file stem of `types/<type>.wcl`, and the folder name under `notes/` |
| Taxonomy name | Same rule as a type name | The file stem of `taxonomies/<taxonomy>.wcl` |
| Task name | Same rule as a type name | The file stem of `tasks/<task>.wcl` |
| Script name | Same rule as a type name | The file stem of `scripts/<script>.ws` |
| Device platform | `linux` or `android` | The file stem of `devices/<platform>.wcl` |
| Field name | Same rule as a type name. `title`, `created`, `updated` and `archived` are reserved | The label of a `field` block |
| Template, view, canvas item name | `[A-Za-z_][A-Za-z0-9_]*` | A block label |
| Note id | A ULID | The file stem of `notes/<type>/<ULID>.wcl` |
| Asset id | A ULID | The folder name of `assets/<ULID>/` |

A file never states its own name, type or id. The path is the only source, so moving a note to a different folder is not a supported edit (D28).

## The data-only subset

Vault files are data. They contain no expressions and no imports (D20). `store` rejects a submitted file that leaves this subset and names the first offending item. The subset is:

```
file     = item*
item     = field | block
field    = name "=" value
block    = kind label* [ "{" item* "}" ]
label    = string | bare-label
value    = string | heredoc | number | "true" | "false" | list | record
list     = "[" [ value ( "," value )* [ "," ] ] "]"
record   = "{" [ key ":" value ( "," key ":" value )* [ "," ] ] "}"
```

- `name`, `kind` and `key` are identifiers: `[A-Za-z_][A-Za-z0-9_]*`.
- A `bare-label` is an identifier that may also contain `-` and `/`.
- A `string` is a plain double-quoted UTF-8 string. The only escapes are `\"`, `\\`, `\n`, `\t` and `\r`.
- A `heredoc` is `<<TAG` or the raw form `<<'TAG'`, closed by a line holding only `TAG`.
- A `number` is a plain decimal: an optional `-`, digits, an optional fraction, an optional exponent. It has no width suffix, unit, radix prefix or `_` separator.
- Labels end at the end of the line. A block with no body is written without braces.
- Items are separated by whitespace. The canonical form puts one item on each line.
- `#` and `//` comments are accepted on input and dropped on every write (D21).

Everything else in WCL is outside the subset and is rejected:

| Rejected | Example |
|----------|---------|
| Imports | `import "./other.wcl"` |
| `let` items | `let base = 3` |
| Declarations | `type`, `interface`, `union`, `symbol_set`, `namespace`, `use`, `connection`, `fn` |
| Decorators | `@doc("...")` |
| Operators and calls | `1 + 2`, `len(tags)`, `!done` |
| References | `estimate = word_count` |
| Interpolation | `$"Hi ${name}"`, `$<<END` |
| Typed literals | `3u32`, `5MiB`, `0xFF`, `utf8"x"` |
| `none` and symbols | `due = none`, `kind = :table` |
| Tables and connection statements | `rows:` followed by `\| ... \|`, `a -> b` |

Leave out an optional field rather than writing `none`.

### Canonical form

Every write emits the canonical form (D21). `wcl_lang`'s formatter fixes indentation, spacing and the heredoc tag. This document fixes what the formatter cannot:

- Fields come before blocks at every level.
- A note's fields follow the order in [Note files](#note-files).
- A definition's fields and blocks follow the order of the tables in this document.
- An optional definition flag that is `false` (`required`, `computed`, `multiple`, `paused`) is left out. A note's `bool` field is always written, `false` included.
- A field that holds its definition's default is still written.
- A `float` is written with a decimal point (`3.0`). An `int` is written without one.
- Block order is significant and is kept: content blocks, list items, ink strokes, canvas nodes, terms, tabs and slots are all stored in display order.

## Value encodings

WCL has no date, URL or reference type. TheVault stores these as strings and its `schema` component validates them (D23).

| Encoding | WCL form | Rule | Example |
|----------|----------|------|---------|
| String | string or heredoc | Any UTF-8 text | `"Ship the importer"` |
| Integer | number | No fraction, fits in a signed 64-bit integer | `3` |
| Float | number | IEEE 754 double. Canonical form has a decimal point | `4.5` |
| Boolean | `true` / `false` | | `false` |
| Date | string | ISO 8601 calendar date, `YYYY-MM-DD` | `"2026-11-01"` |
| Datetime | string | ISO 8601 date and time with seconds and a required offset: `YYYY-MM-DDTHH:MM:SS[.fff](Z\|±HH:MM)`. The writer keeps the offset it was given | `"2026-10-20T09:30:00+11:00"` |
| Date range | record | `{ start: <date>, end: <date> }`, or two datetimes. Both ends inclusive, `end` not before `start`. Both ends have the same form, never one date and one datetime | `{ start: "2026-10-20", end: "2026-11-01" }` |
| Duration | string | ISO 8601 duration | `"PT1M"` |
| URL | string | An absolute URL with a scheme | `"https://example.com/importer"` |
| Note reference | string | The target note's ULID: 26 Crockford base32 characters, upper case | `"01JB8Z4K2MQ7V3X9T5R6W8Y2ZB"` |
| Asset reference | string | The asset's ULID, naming the folder `assets/<ULID>/` | `"01JB9A0C3D4E5F6G7H8J9K0M1N"` |
| Taxonomy term | string | The term's path from its root, names joined by `/` | `"Active/In progress"` |
| Geo location | record | `{ lat: <float>, lng: <float> }` in WGS 84 decimal degrees. `lat` in -90..90, `lng` in -180..180 | `{ lat: -37.8136, lng: 144.9631 }` |
| Image location | record | `{ x: <float>, y: <float> }` in pixels from the map image's top-left corner | `{ x: 412.0, y: 188.5 }` |
| Palette colour | string | One of `gray`, `brown`, `orange`, `yellow`, `green`, `blue`, `purple`, `pink`, `red` (OQ-S4) | `"blue"` |
| Ink colour | string | `#rrggbb`, lower-case hex, or `ink`, the default ink, which draws in the theme's foreground | `"#2563eb"`, `"ink"` |

A record must be written with every member. `schema` rejects a missing or extra key, because WCL does not.

## Type files

`types/<type>.wcl` defines one type: its fields, its named templates, and the handlers that run on its notes. Fields are `field` blocks, templates are `template` blocks, and handlers are `handler` blocks.

| Item | Form | Required | Meaning |
|------|------|----------|---------|
| `label` | string | yes | Display name, singular |
| `plural` | string | no | Display name, plural. Defaults to `label` |
| `description` | string | no | Shown in the type picker and returned by schema inspection |
| `icon` | string | no | Icon name, resolved by the web client. An unknown name shows a default icon |
| `colour` | palette colour | no | Accent colour for the type |
| `field <name> { ... }` | block | no | One field. Order is display and canonical order |
| `template <name> { ... }` | block | yes, `default` | One named template |
| `handler "<event>" { ... }` | block | no | One handler. Handlers for one event run in file order |

```wcl
label       = "Board"
plural      = "Boards"
description = "A note that is mostly one canvas."
icon        = "layout-dashboard"

field board {
  kind  = "canvas"
  label = "Board"
}

template default {
  tab "Board" {
    slot "title"
    slot "board"
  }
}
```

### Attributes a reader does not know

A reader tolerates an attribute of a type, or of a field, that it does not know. This keeps a type file from a newer server readable in an older app, so that a release can add an attribute without raising the format version. The rule covers the items at the top of a type file, the items of a `field` block, including an item that belongs to another kind, such as `accept` on a `string` field, and every attribute inside a template: of the `template` block, of a `tab`, of a `slot`, and of a fixed content block in a tab, such as `tone` on a `paragraph`:

- The reader ignores the attribute. The type loads, and notes of the type are written and checked against what the reader knows.
- The reader reports the attribute as a warning, `<path>: is not an attribute of <owner> in this version; it is ignored`. `GET /api/types` and `GET /api/schema` list each type's `warnings`, the JSON answer of `PUT /api/types/<name>` carries them, and the type editor shows them, so a misspelt attribute is seen. Each warning is `{ "at", "message", "severity": "warning" }`.
- The file keeps the attribute. Saving the type from the type editor rewrites the file from what the editor knows, which leaves the attribute out.

Everything else stays an error that refuses the type: a malformed value of an attribute the reader knows, such as `accept = "video"` or `required = "yes"`, a missing required attribute, a `kind` the reader does not know, an unknown block, inside a template or anywhere else, and an unknown handler item. A change that an older reader would misread rather than ignore still raises the format version (D27).

A release that adds an attribute is rolled out apps first. See [Releasing a format change](android-release.md#releasing-a-format-change).

### Built-in types

Every vault has nine types that the app relies on. The server writes each one's type file when it starts on a vault that lacks it, so a new vault and a vault made before a type existed both get it, and adds to a built-in type the vault has every field its shipped default gained since: see [Shipped defaults and upgrades](#shipped-defaults-and-upgrades). `vault init --starter` writes them too. The files it writes are in `crates/writer/defaults/`, one per type, named after it. A built-in type can be customised like any other type, but it cannot be deleted: a delete answers 409 `built_in_type`. Schema inspection marks each with `"builtin": true`.

| Type | Label | For | Must keep |
|------|-------|-----|-----------|
| `dailynote` | Daily note | One note for each day. The app opens today's note and finds a day's note by its date | `date`, one required `date` field |
| `fleeting` | Fleeting note | A quick capture of text or images, made from the app's capture dialog. Unprocessed ones wait in the Inbox (`from fleeting where !processed`) until they are marked done, converted into a note of another type, or deleted | `processed`, one `bool` field that handlers do not compute |
| `message` | Message | One SMS, MMS or RCS message from a phone, written by the default device scripts (D15) | nothing |
| `call` | Call | One phone call, made, received or missed (D15) | nothing |
| `conversation` | Conversation | One message thread. Its page lists the thread's messages (D15) | nothing |
| `email` | Email | One Gmail message, written by the Gmail sync (google-sync D17) | nothing |
| `email_thread` | Email thread | One Gmail thread. Its page lists the thread's emails (google-sync D4) | nothing |
| `event` | Event | One Google Calendar event, or one occurrence of a recurring one, written by the calendar sync (google-sync D15) | nothing |

A change that removes a "must keep" field, or changes its kind, is refused with `invalid_type`.

The phone data types, `email`, `email_thread` and `event` reference `person`, which is a starter type and not a built-in one. When the server adds any of them to a vault that has no `person` type, it adds `crates/writer/defaults/person.wcl` with them, in the same commit. That `person` can be customised and, once nothing references it, deleted. A vault that already has a `person` type keeps it, and gains on start each field of `crates/writer/defaults/person.wcl` it lacks, such as `emails`, `phones`, `addresses` and `device_contacts`, which the device scripts write.

The fields of the phone data types, as a new vault gets them:

| Type | Field | Kind | Meaning |
|------|-------|------|---------|
| `message` | `channel` | `select`: `sms`, `mms`, `rcs` | How the message travelled |
| | `direction` | `select`: `in`, `out` | Received or sent |
| | `from` | `reference` to `person` | The sender, when a person matches |
| | `to` | `reference` to `person`, `multiple` | The recipients that match a person |
| | `from_raw` | `string` | The sender's number or display name, as the phone gave it |
| | `to_raw` | `list` of `string` | Each recipient's number or display name, as the phone gave it |
| | `conversation` | `reference` to `conversation` | The thread the message belongs to |
| | `sent` | `datetime` | When it was sent |
| | `body` | `content` | The text |
| | `attachments` | `asset`, `multiple` | MMS attachments. The default template shows the ones that are images on the note page and its card |
| | `partial` | `bool`, default `false` | Captured from a notification, so it may be incomplete (RCS) |
| | `processed` | `bool`, default `false` | Set when the user, or an agent, has dealt with the message. Its slot sits beside `from` on the page and on the card. No device script writes it, so a message captured again keeps it |
| | `device_ref` | `string` | The phone's id for the message: a provider row id or a notification key. The device scripts use it to capture a message once |
| `call` | `direction` | `select`: `in`, `out`, `missed` | |
| | `person` | `reference` to `person` | The other party, when a person matches |
| | `number` | `string` | The other party's number |
| | `started` | `datetime` | When the call started |
| | `duration` | `int` | Its length in seconds |
| | `device_ref` | `string` | The phone's call log id, so a call is captured once |
| `conversation` | `participants` | `reference` to `person`, `multiple` | The people in the thread |
| | `channel` | `select`: `sms`, `mms`, `rcs` | |
| | `thread_ref` | `string` | The phone's id for the thread |

A note with no `processed` value, such as one made before 0.1.13 added the field, counts as unprocessed: a missing field is false in a query, so `from email where !processed` and `from message where !processed` list the unprocessed ones, both those with `processed = false` and those without the field.

The `conversation` type's default template shows its messages, oldest first:

```wcl
template default {
  tab "Messages" {
    slot "participants"
    query_view {
      title = "Messages"
      query = "from message where conversation == this sort sent asc"

      view table {
        kind    = "table"
        columns = ["sent", "direction", "from_raw", "title"]
      }
    }
  }
}
```

The fields of the Google sync types, as a new vault gets them. The Gmail and calendar syncs link a sender, recipient or attendee to the person whose `emails` holds the address, ignoring case, and create no person (google-sync D5):

| Type | Field | Kind | Meaning |
|------|-------|------|---------|
| `email` | `from` | `reference` to `person` | The sender, when a person matches. The title is the subject |
| | `from_raw` | `string` | The sender as the message gave it, such as `Alex Example <alex@example.com>` |
| | `to`, `cc` | `reference` to `person`, `multiple` | The recipients that match a person |
| | `to_raw`, `cc_raw` | `list` of `string` | Each recipient as the message gave it |
| | `sent` | `datetime` | When it was sent |
| | `thread` | `reference` to `email_thread` | The thread it belongs to |
| | `labels` | `taxonomy` `gmail_label`, `multiple` | Its Gmail labels |
| | `snippet` | `string` | Gmail's short preview |
| | `body` | `content` | The text |
| | `html` | `asset` | The HTML part, as Gmail sent it |
| | `attachments` | `asset`, `multiple` | Its attachments |
| | `processed` | `bool`, default `false` | Set when the user, or an agent, has dealt with the email. Its slot sits beside `from` on the page and on the card. The Gmail sync never writes it: a label change rewrites only `labels` |
| | `gmail_ref` | `string`, hidden | Gmail's message id, so a message is pulled once |
| `email_thread` | `participants` | `reference` to `person`, `multiple` | The people in the thread. The title is the subject |
| | `thread_ref` | `string`, hidden | Gmail's thread id |
| `event` | `when` | `date_range`, `of = "date_or_datetime"` | Dates for an all-day event, datetimes with their offsets for a timed one |
| | `calendar` | `string` | The id of the Google calendar it is on, as the Google settings' `calendars` lists it, such as `primary` |
| | `location` | `string` | |
| | `description` | `content` | |
| | `attendees` | `reference` to `person`, `multiple` | The attendees that match a person |
| | `attendees_raw` | `list` of `string` | Each attendee's address as Google gave it |
| | `organiser` | `string` | |
| | `status` | `select`: `confirmed`, `tentative`, `cancelled` | |
| | `meet_url` | `url` | The Google Meet link |
| | `series` | `string`, computed | The recurrence rule of the series an occurrence belongs to, as text |
| | `google_ref` | `string`, hidden | Google's id for the event, or for this occurrence |
| | `etag` | `string`, hidden | Google's version of the event, as last synced |
| | `snapshot` | `dict` of `string`, hidden | The synced fields as Google last sent them, so a sync can tell which side changed (google-sync D10) |

The `event` type declares three handlers, on `after_create`, `after_update` and `after_delete`, each running `scripts/google_event.ws` with `allow_domains = ["oauth2.googleapis.com", "www.googleapis.com"]` and the secrets `google_client_id`, `google_client_secret`, `google_refresh_token` and `google_settings`. They write the vault's changes to an event note back to Google Calendar (google-sync D9, D10, D20); `docs/scripts.md`, "The Google Calendar write-back", says what each sends.

The `email_thread` type's default template lists its emails with the query `from email where thread == this sort sent asc`.

`email` uses the built-in open taxonomy `gmail_label`, at `taxonomies/gmail_label.wcl` (google-sync D14). The server adds it when it starts on a vault that lacks it, in the same commit as the types, and never changes it afterwards: the Gmail sync adds each label it meets. A nested Gmail label is a term path, such as `Work/Clients`. Gmail's own labels sit under the `system` term the shipped file holds: `system/INBOX`, `system/UNREAD`, `system/STARRED`, `system/IMPORTANT`, `system/SENT` and `system/DRAFT`.

The Google sync has no settings note type: its settings and its progress are secrets, edited on Settings > Integrations and written by the sync's own scripts with `secrets::set`, so a run writes no note history for them (google-sync D21). `docs/scripts.md`, "The Google sync's secrets", lists each one and its JSON shape. Vaults from 0.1.7, which had a built-in `google_sync` type, lose it in upgrade step `0004_remove_google_sync` (see [Shipped defaults and upgrades](#shipped-defaults-and-upgrades)).

The Gmail sync is the shipped task `tasks/google_gmail.wcl`, which runs `scripts/google_gmail.ws` every 10 minutes with a five-minute timeout, reaching `oauth2.googleapis.com`, `gmail.googleapis.com` and `www.googleapis.com` with the secrets `google_client_id`, `google_client_secret`, `google_refresh_token`, `google_settings` and `google_gmail_state` (google-sync D19, D21). It ships paused; connecting Google unpauses it, and Run now works while it is paused. Each run:

- Reads the settings from the `google_settings` secret, the defaults where it is missing, and its state from `google_gmail_state`. With no `google_refresh_token`, Google is not connected: it logs that and ends successfully.
- Refreshes an access token at the token endpoint and stores none. When Google answers `invalid_grant`, the run writes the error to the state's `last_error`, adds the bell notification "Google sign-in expired" unless `notified_invalid_grant` is already true, sets it, and ends successfully, so the task-failure notification does not fire as well (google-sync D18). Any other failure is written to `last_error` and fails the run. A run that succeeds clears it and writes `last_run`.
- On the first run, lists the window: the last `gmail_window_days` days, of each label in `gmail_labels` (matched to Gmail's labels by name or id, without case) in `labels` mode, or of all mail in `all` mode (google-sync D11). Later runs read Gmail's history from the state's `history_id`: a new message in scope is pulled, a label change rewrites the email's `labels` and nothing else, and a message deleted in Gmail archives its note (google-sync D9). When Gmail no longer has that history (404), the run lists the window again.
- With `gmail_backfill_to` set, also pulls one listing page of mail older than the window, down to that date, keeping its place in the state's `backfill_page` (`<listing>:<page token>`, then `done:<date>` once the date is reached; a new date starts again).
- Pulls each message once, by `gmail_ref`, archived notes included. The note's `body` is the `text/plain` part as paragraphs, or else the `text/html` part turned into text, with inline markup characters escaped. The HTML part is stored as the asset `<message id>.html` in `html`. Attachments up to 700 KB are stored in `attachments`, and larger ones are listed by name and size in a paragraph. A message whose response is over the 1 MiB a script may read keeps its headers, labels and snippet, and a paragraph saying the body was too large (google-sync D3). A body part is decoded up to 120,000 base64 characters, and a paragraph says when the rest was cut.
- Finds or creates the `email_thread` with the message's `thread_ref`, titled by the subject without `Re:` or `Fwd:`, and adds any new participants (google-sync D4). `from`, `to` and `cc` link to the person whose `emails` hold the address; an unknown address stays only in `from_raw`, `to_raw` or `cc_raw`, and no person is created (google-sync D5).
- Adds each label it meets to `gmail_label`, a user label by its name and a Gmail label under `system`.
- Stops early, before four minutes or before it has decoded 1,200,000 base64 characters, so it keeps within its timeout and its instruction limit. It then leaves its cursor where it was, and the next run carries on: messages already pulled are skipped.

The `person` type, both the starter pack's and the one added with the phone data types, holds every detail a phone contact holds, and the device scripts keep the two in step both ways (D16, D24):

```wcl
field emails {
  kind   = "list"
  label  = "Emails"
  items  = "string"
  labels = "email_labels"
}

field email_labels {
  kind        = "list"
  label       = "Email labels"
  description = "The label of each email address, in the same order: mobile, home, work, main, other, or the phone's own label."
  items       = "string"
}

field phones {
  kind        = "list"
  label       = "Phones"
  description = "Phone numbers in international (E.164) form, such as +61400000000."
  items       = "string"
  labels      = "phone_labels"
}

field phone_labels {
  kind        = "list"
  label       = "Phone labels"
  description = "The label of each phone number, in the same order: mobile, home, work, main, other, or the phone's own label."
  items       = "string"
}

field addresses {
  kind        = "list"
  label       = "Addresses"
  description = "Postal addresses, each on one line with its lines joined by commas."
  items       = "string"
  labels      = "address_labels"
}

field address_labels {
  kind        = "list"
  label       = "Address labels"
  description = "The label of each postal address, in the same order: home, work, other, or the phone's own label."
  items       = "string"
}

field organisation {
  kind  = "string"
  label = "Organisation"
}

field job_title {
  kind  = "string"
  label = "Job title"
}

field nickname {
  kind  = "string"
  label = "Nickname"
}

field birthday {
  kind        = "string"
  label       = "Birthday"
  description = "YYYY-MM-DD, or --MM-DD when the year is not known."
}

field websites {
  kind  = "list"
  label = "Websites"
  items = "url"
}

field contact_note {
  kind        = "string"
  label       = "Contact note"
  description = "The note on the phone contact."
}

field photo {
  kind   = "asset"
  label  = "Photo"
  accept = "image"
}

field device_contacts {
  kind        = "dict"
  label       = "Device contacts"
  description = "The phone contact linked to this person, by device id."
  values      = "string"
  hidden      = true
}
```

`phones`, `emails` and `addresses` each have a label list beside them, index-aligned and named by their [`labels`](#list): the label of `phones[0]` is `phone_labels[0]`. The default template places `photo` first, then the name, and no slot of a label list; the card shows the photo and the title first. A label is `mobile`, `home`, `work`, `main`, `other`, or the phone's custom label text, and `""` for none. `phones` are E.164, so `has(phones, "+61400000000")` finds the person by a number. `addresses` are postal addresses, each the phone's formatted address with its line breaks joined by `", "`. `birthday` is `YYYY-MM-DD`, or `--MM-DD` when the year is not known. `contact_note` is the note on the phone contact, kept apart from `body`, which is the person's own content. `photo` is the contact photo. The starter's `person` also has `homepage` and a Work tab.

A vault from before people carried every contact detail has `addresses` holding email addresses and a separate single `email`. The upgrade step `0001_person_emails` renames that `addresses` to `emails`, moves a non-empty `email` value to the front of `emails` (an equal address further on, ignoring case, is dropped), and removes `email`, as one planned type change that edits every person note it applies to; see [Shipped defaults and upgrades](#shipped-defaults-and-upgrades). The template slots of `email` and `addresses` become one slot of `emails`. Fields the user added stay, and the seeding that follows adds the other new fields, postal `addresses` among them. A device's local vault adds no field to such a `person` type, and waits for the home server's step to sync.

A vault from before values and their labels were edited together has `emails`, `phones` and `addresses` with no `labels`, and slots for the three label lists in the default template. The upgrade step `0002_person_labels` gives each of the three that the type has its `labels`, adding the shipped label list after it when the type lacks one; gives `photo` `accept = "image"` when every photo in the vault is an image asset; and removes the slots of `email_labels`, `phone_labels` and `address_labels` from the `default` template. It is one planned type change that edits no note, and it changes nothing else the user changed in the type. It does nothing to a `person` type where any field already has `labels`.

`device_contacts` maps each device to the id of the contact on that device that is linked to the person. The key is `device_` followed by the device id, as in `device_01JB8Z4K2MQ7V3X9T5R6W8Y2ZB`, because a dict key must be an identifier and a device id starts with a digit. It is hidden: the device scripts write it, and the app does not show it.

```wcl
dict device_contacts {
  device_01JB8Z4K2MQ7V3X9T5R6W8Y2ZB = "4127"
}
```

The `fleeting` type as a new vault gets it. Its page shows the images and the body; `processed` is set from the Properties panel or the Inbox:

```wcl
label       = "Fleeting note"
plural      = "Fleeting notes"
description = "A quick capture of text or images, waiting in the inbox to be processed."
icon        = "inbox"
colour      = "yellow"

field body {
  kind  = "content"
  label = "Body"
}

field image {
  kind     = "asset"
  label    = "Images"
  multiple = true
}

field processed {
  kind    = "bool"
  label   = "Processed"
  default = false
}

template default {
  tab "Capture" {
    image {
      field = "image"
    }
    slot "body"
  }
}

template card {
  tab "Card" {
    image {
      field = "image"
    }
    slot "title"
  }
}
```

The capture dialog titles a fleeting note with the first line of its text, cut at a word to at most 80 characters, or `Image <local date and time>` when it holds only images. It writes the text into the type's `body` field (else its first `content` field) and the images into `image` (else its first `asset` field). Converting a fleeting note into another type copies the title, puts the body in the target's first `content` field and the images in its first `asset` field (only the first image when that field is not `multiple`), and moves the fleeting note to the trash.

### Field definitions

Every `field` block takes these items. The next section lists the extra items each kind takes.

| Item | Form | Default | Meaning |
|------|------|---------|---------|
| `kind` | string | required | One of the kinds in [Field kinds](#field-kinds) |
| `label` | string | the field name | Display label |
| `description` | string | none | Help text |
| `required` | bool | `false` | A note must have a value. A newly added required field must have a `default` (D6) |
| `default` | value | none | The value a new note gets, and the value a schema change writes into existing notes. It uses the note encoding of the field's kind |
| `computed` | bool | `false` | Only handlers write the field. The UI shows it read-only, and a client write is rejected (D3). A computed field cannot also be `required` |
| `hidden` | bool | `false` | The properties sidebar does not show the field. The value is still stored, validated, queried, and written by the API and scripts, and a slot in a template still shows it. A hidden field cannot also be `required` |
| `multiple` | bool | `false` | Allowed on `select`, `taxonomy`, `reference` and `asset`. The note holds a list |

`default` is allowed on `string`, `int`, `float`, `bool`, `date`, `datetime`, `url`, `select`, `taxonomy` and `list` fields.

## Field kinds

Each kind below gives its definition items and its note encoding. A field holding several values (`multiple = true`, or kind `list`) is a WCL list in the note. The order of that list is kept.

Most kinds are written as a field, `name = value`, in the note. `content`, `dict`, `ink` and `canvas` are written as a block whose label is the field name, as in `content body { ... }`. They are called compound fields below.

### `string`

Free text. Inline markup is not interpreted in a string field.

```wcl
field subtitle {
  kind  = "string"
  label = "Subtitle"
}
```

```wcl
subtitle = "Capacities first, then everything else"
```

### `int`

A signed 64-bit integer.

```wcl
field estimate {
  kind     = "int"
  required = true
  default  = 1
}
```

```wcl
estimate = 3
```

### `float`

A 64-bit float. A note may write an integer literal, which reads as the same float. The canonical form writes the decimal point.

```wcl
field score {
  kind = "float"
}
```

```wcl
score = 4.5
```

### `bool`

```wcl
field done {
  kind    = "bool"
  default = false
}
```

```wcl
done = false
```

### `date`

```wcl
field due {
  kind = "date"
}
```

```wcl
due = "2026-11-01"
```

### `datetime`

```wcl
field starts_at {
  kind = "datetime"
}
```

```wcl
starts_at = "2026-10-20T09:30:00+11:00"
```

### `date_range`

| Item | Values | Default |
|------|--------|---------|
| `of` | `"date"`, `"datetime"` or `"date_or_datetime"` | `"date"` |

```wcl
field span {
  kind = "date_range"
  of   = "date"
}
```

```wcl
span = { start: "2026-10-20", end: "2026-11-01" }
```

`of = "date"` takes two dates and `of = "datetime"` two datetimes. `of = "date_or_datetime"` takes
either, one value at a time: both ends are dates, or both ends are datetimes. A value with one date
end and one datetime end fails, and so does an `end` before its `start`. An event that lasts all
day holds dates, and a timed event on the same field holds datetimes:

```wcl
field when {
  kind = "date_range"
  of   = "date_or_datetime"
}
```

```wcl
when = { start: "2026-10-20", end: "2026-10-21" }
```

```wcl
when = { start: "2026-10-20T09:30:00+11:00", end: "2026-10-20T10:00:00+11:00" }
```

Queries read each value by its form. A date compares and sorts as the start of its day in UTC, so
`when.start >= "2026-10-20"` and `sort when.start` order dated and timed values on one line. The
`calendar` and `timeline` views place a dated value as an all-day span and a timed value at its
times. No control fits such a field: the `calendar` control picks days only.

### `url`

```wcl
field homepage {
  kind = "url"
}
```

```wcl
homepage = "https://example.com/importer"
```

### `select`

A choice from a fixed list of strings defined on the field.

| Item | Form | Meaning |
|------|------|---------|
| `options` | list of strings | Required. The allowed values, in display order |

```wcl
field priority {
  kind    = "select"
  options = ["low", "medium", "high"]
  default = "medium"
}

field channels {
  kind     = "select"
  multiple = true
  options  = ["email", "chat", "phone"]
}
```

```wcl
priority = "high"
channels = ["email", "chat"]
```

### `taxonomy`

One term, or several, from a global taxonomy (D7). A note stores the term's path. A kanban view requires a single-term field.

| Item | Form | Meaning |
|------|------|---------|
| `taxonomy` | string | Required. The taxonomy name, a file stem in `taxonomies/` |

```wcl
field status {
  kind     = "taxonomy"
  taxonomy = "status"
  required = true
  default  = "Inbox"
}

field topics {
  kind     = "taxonomy"
  taxonomy = "topic"
  multiple = true
}
```

```wcl
status = "Active/In progress"
topics = ["Software/Rust", "Writing"]
```

A term path names any term, not only a leaf. Renaming or moving a term rewrites every note that holds it, in one commit.

### `reference`

A link to another note by ULID (D4, D5).

| Item | Form | Default | Meaning |
|------|------|---------|---------|
| `target` | string | required | The target type name. `"*"` allows any type, and only with `relation = "one_way"` |
| `relation` | string | `"one_way"` | `one_way`, `two_way` or `peer` |
| `inverse` | string | none | Required for `two_way`: the field on the target type that holds the other side |
| `on_delete` | string | `"unlink"` | What happens to this note when a target is deleted: `unlink`, `restrict` or `cascade` |

The relations:

- `one_way` is stored only on this note.
- `two_way` is stored on both notes. The `inverse` field must be a `two_way` reference field on the target type whose own `inverse` names this field. Either side may be single or `multiple`. The writer updates both files in one commit.
- `peer` is a symmetric group. The field must be `multiple` and its `target` must be this type. Every member stores the list of the other members, so adding a note to one member's list adds it to every member's list.

`on_delete` is read from the field on the note that holds the reference. Take a note A whose field points at note B, and delete B:

- `unlink` removes B from A's field, and A's update handlers run.
- `restrict` refuses the delete and lists A.
- `cascade` deletes A as well, and its delete handlers run.

```wcl
field board {
  kind      = "reference"
  target    = "board"
  on_delete = "restrict"
}

field parent {
  kind      = "reference"
  target    = "specimen"
  relation  = "two_way"
  inverse   = "children"
  on_delete = "cascade"
}

field children {
  kind     = "reference"
  target   = "specimen"
  relation = "two_way"
  inverse  = "parent"
  multiple = true
}

field related {
  kind     = "reference"
  target   = "specimen"
  relation = "peer"
  multiple = true
}
```

```wcl
board    = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZF"
parent   = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB"
children = ["01JB8Z4K2MQ7V3X9T5R6W8Y2ZC"]
related  = ["01JB8Z4K2MQ7V3X9T5R6W8Y2ZD", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZE"]
```

### `list`

An ordered list of scalar values of one kind, or of taxonomy terms.

| Item | Form | Meaning |
|------|------|---------|
| `items` | string | Required. One of `string`, `int`, `float`, `bool`, `date`, `datetime`, `url` and `taxonomy` |
| `taxonomy` | string | Required with `items = "taxonomy"`, and allowed only with it. The taxonomy name, a file stem in `taxonomies/` |
| `labels` | string | Allowed only with `items = "string"`. Another field of the type, a list of strings, that holds one label per value in the same order |

```wcl
field aliases {
  kind  = "list"
  items = "string"
}
```

```wcl
aliases = ["importer", "capacities import"]
```

A list with `items = "taxonomy"` holds term paths. It is a `taxonomy` field with `multiple = true` in every respect but how it is declared: a note writes the same list, values are checked against the taxonomy the same way, a term can appear once, queries such as `has()` and `under()` treat it the same, and a kanban view cannot group by it. A `default` must hold terms the taxonomy already has. `multiple` is not allowed on a list.

```wcl
field tags {
  kind     = "list"
  items    = "taxonomy"
  taxonomy = "tags"
}
```

```wcl
tags = ["Gardening", "Gardening/Roses"]
```

A list of strings with `labels` names a second list that labels its values, index-aligned: the label of `phones[0]` is `phone_labels[0]`, and `""` is no label. The two lists are edited together: the app shows one chip per value with its label before it, each with Copy, Edit and Remove, and writes both lists in one save, so a value may appear twice and a label stays with its value. The field `labels` names is not shown on its own: the properties sidebar leaves it out and the note page shows no slot for it, even when a template places it. Cards and tables show the pairs as text, such as `mobile +61400000020, home +61400000021`. The schema rejects `labels` that names a field the type lacks ("the type has no field ..."), a field that is not a list of strings ("... must be a list of strings, and it is a ... field"), the field itself ("cannot name the field itself"), or a field another field already names ("... already holds the labels of field ...").

```wcl
field phones {
  kind   = "list"
  items  = "string"
  labels = "phone_labels"
}

field phone_labels {
  kind  = "list"
  items = "string"
}
```

```wcl
phones       = ["+61400000020", "+61400000021"]
phone_labels = ["mobile", "home"]
```

### `dict`

A set of named values of one kind. Keys are identifiers (`[A-Za-z_][A-Za-z0-9_]*`), because WCL record keys and block fields must be. The note writes a `dict` block labelled with the field name, with one field per key. Keys are written in the order they were added.

| Item | Form | Meaning |
|------|------|---------|
| `values` | string | Required. One of `string`, `int`, `float`, `bool`, `date`, `datetime`, `url` |

```wcl
field extra {
  kind   = "dict"
  values = "string"
}
```

```wcl
dict extra {
  owner  = "Wil"
  source = "capacities"
}
```

### `location`

A pin on a real-world map or on an image map.

| Item | Form | Meaning |
|------|------|---------|
| `map` | string | Required. `"geo"` for a real-world map, `"image"` for an image map |
| `image` | asset reference | Required when `map = "image"`. The image the pins sit on |

```wcl
field place {
  kind = "location"
  map  = "geo"
}

field desk {
  kind  = "location"
  map   = "image"
  image = "01JB9A0C3D4E5F6G7H8J9K0M1Q"
}
```

```wcl
place = { lat: -37.8136, lng: 144.9631 }
desk  = { x: 412.0, y: 188.5 }
```

### `content`

A sequence of [content blocks](#content-blocks) (D34). The note writes a `content` block labelled with the field name.

```wcl
field body {
  kind = "content"
}
```

```wcl
content body {
  heading {
    level = 2
    text  = "Plan"
  }

  paragraph {
    text = "Import from **Capacities** first."
  }
}
```

### `asset`

An uploaded file, by asset ULID.

```wcl
field cover {
  kind = "asset"
}

field attachments {
  kind     = "asset"
  multiple = true
}
```

```wcl
cover       = "01JB9A0C3D4E5F6G7H8J9K0M1N"
attachments = ["01JB9A0C3D4E5F6G7H8J9K0M1P"]
```

| Item | Form | Meaning |
|------|------|---------|
| `accept` | string | `"image"` or `"any"`. Default `"any"`. With `"image"`, a value must be an asset whose file is an image, by its file name (`.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.avif` or `.svg`); validation rejects another asset with "asset ... is "...", and this field holds only images" |

A single asset field with `accept = "image"` is edited as a square image tile: the image, or a placeholder ("Add photo" for a field labelled Photo, else "Add image") when it is empty. The tile opens a dialog that uploads an image, by the file picker or by dropping one, or picks one of the vault's images; a set tile also offers Remove and Open full size. Cards and tables show the image as a thumbnail. Any other asset field keeps the picker of uploaded files.

```wcl
field photo {
  kind   = "asset"
  accept = "image"
}
```

### `ink`

A drawing. The note writes an [`ink`](#ink) block labelled with the field name. An ink field and an ink content block have the same format.

```wcl
field sketch {
  kind = "ink"
}
```

```wcl
ink sketch {
  width  = 400
  height = 400

  stroke {
    tool   = "pen"
    colour = "#2563eb"
    size   = 3.0
    points = [100.0, 100.0, 0.5, 200.0, 150.0, 0.8, 300.0, 100.0, 0.5]
  }
}
```

### `canvas`

A canvas. The note writes a [`canvas`](#canvas) block labelled with the field name. A canvas field and a canvas content block have the same format. A template places a canvas field like any other field, which is how a type gives every note a canvas of its own (user story 11).

```wcl
field board {
  kind = "canvas"
}
```

```wcl
canvas board {
  viewport {
    x    = 0.0
    y    = 0.0
    zoom = 1.0
  }
}
```

## Templates

A type has two templates at most, `default` and `card` (D14). Each lays fields and content blocks out on tabs, and either can have as many tabs as it needs. The schema rejects a template with any other name.

| Name | Used by | When missing |
|------|---------|--------------|
| `default` | The note page and details views | Required |
| `card` | Cards, kanban and canvas views, map pins, and canvas note nodes | Falls back to the first tab of `default` |

A template takes these items:

| Item | Form | Meaning |
|------|------|---------|
| `label` | string | Optional display name |
| `tab "<title>" { ... }` | block | At least one tab. A template with one tab shows no tab bar. Card views render only the first tab that shows for the note |

A tab holds an optional `when` and an ordered mix of slots and content blocks:

| Item | Form | Meaning |
|------|------|---------|
| `when` | string | Optional. A condition on the note shown, in the syntax of a query's `where` clause. The tab shows only for a note that meets it. See [Conditional tabs](#conditional-tabs) |
| `slot "<field>"` | block | Places a field. `<field>` is a field of the type or `title`. A field appears at most once per template |
| `label` inside a slot | string | Overrides the field's label in this template |
| any [content block](#content-blocks) | block | Fixed content shown on every note. In a `query_view`, `this` is the note being shown. An `image` may show an asset field of the note instead of a fixed asset: see [Images from fields](#images-from-fields) |
| `columns { ... }` | block | A [columns](#columns) layout whose columns hold slots and fixed blocks, so fields sit side by side |
| `control { ... }` | block | A [control](#control) that edits a field in place, in the tab or in a column. A field may have a slot and any number of controls |

A slot inside a column counts as placing its field, as one directly in the tab does, and the rule of one slot per field covers the whole template, columns included. Controls in columns are checked as anywhere else. `{{name}}` references in fixed blocks inside columns are checked as anywhere else. Inside a tab, the canonical form writes a `columns` block and its columns compactly, with no blank lines between their blocks.

```wcl
template default {
  tab "Overview" {
    columns {
      column {
        width = 2
        slot "status"
        slot "due"
      }
      column {
        width = 1
        paragraph {
          text = "Owned by {{owner}}."
        }
        slot "estimate"
      }
    }
    slot "body"
  }
}
```

A field that no slot places is still stored and editable through the API and in the note page's properties sidebar. The template only controls the layout.

### Conditional tabs

A tab with `when` shows only for a note that meets the condition (D14). The condition is an expression in the syntax of a query's `where` clause ([PRD Appendix B](plans/thevault-prd/PRD.md), D42), and it reads the note shown:

- The names are the type's fields and the built-ins `id`, `type`, `title`, `created`, `updated` and `archived`. `this` is the note shown, so `this.status` is `status`.
- The operators are `==`, `!=`, `<`, `<=`, `>`, `>=`, `&&`, `||`, `!`, the arithmetic operators, and parentheses, with the query language's literals and precedence.
- The functions are `has()`, `under()`, `contains()`, `empty()`, `any()`, `all()`, `today()`, `now()` and `days()`. `linked_from()` reads other notes, so a condition refuses it.
- `.` reads the `start` or `end` of a date range, and the members of a location or dict field. It does not follow a reference, since that reads another note.
- A missing value behaves as in a query: it is false in a condition and never compares as less or greater than anything, so `!(due < today())` holds for a note with no `due`. A condition that fails to evaluate, such as `estimate == "3"` on an `int` field, is false.

The schema checks the condition when the type is saved. A condition that does not parse, names a field the type lacks, or reads another note is an error at the tab's path, as `template default > tab "Review".when: `stage` is not a field of the type`.

The web client evaluates the condition on the note as it is edited, so a tab appears or disappears as soon as a field changes, before the note is saved. `today()` is the browser's local date. On the page:

- A tab whose condition is false is not in the tab bar.
- When the open tab becomes hidden, the page moves to the first tab that shows.
- When no tab shows, the first tab shows, so a note never renders empty.

The `card` template follows the same rules: a card renders the first of its tabs that shows for the note, or its first tab when none does.

A condition does not change the rule of one slot per field per template. Two tabs that show for different notes can still show the same field: one places it with a slot, the other with a `{{name}}` reference.

```wcl
template default {
  tab "Overview" {
    slot "title"
    slot "status"
  }

  tab "Review" {
    when = "under(status, \"Active\") && !(due > today() + days(7))"
    slot "due"
    slot "reviewer"
  }
}
```

A vault that uses `when` needs apps at 0.1.10 or later: an older app refuses a tab attribute it does not know. See [Releasing a format change](android-release.md#releasing-a-format-change).

### Field references

The text of a fixed content block in a template can show a field's value inline: `{{name}}` in any `text` item (a heading, paragraph, quote, list item or toggle summary, including one inside a canvas card), in an admonition's `title`, or in an image's `alt` or `caption`, renders the current value of field `name` of the note being shown, read-only. `name` is `title` or a field of the type; the schema rejects any other name with "`{{name}}` names no field of the type". A reference is not a slot: it places nothing, so the field still has no entry on the page and stays editable in the properties sidebar.

- `name` is an identifier written directly between the braces, with no spaces: `{{ due }}` is plain text.
- `\{{name}}`, with the first brace escaped, and `{{name}}` inside a code span are plain text and are not checked.
- `{{name}}` is template-only markup. In a note's own content it is plain text with no meaning.

```wcl
template default {
  tab "Overview" {
    paragraph {
      text = "**{{title}}** is due {{due}}."
    }
    slot "body"
  }
}
```

```wcl
template default {
  label = "Full page"

  tab "Overview" {
    slot "title"
    slot "status"
    slot "estimate" {
      label = "Hours"
    }
    slot "body"
  }

  tab "Relations" {
    slot "children"
    query_view {
      query = "from specimen where parent == this && !done sort due asc"

      view open {
        kind    = "table"
        columns = ["title", "status", "due"]
      }
    }
  }
}

template card {
  tab "Card" {
    slot "title"
    slot "status"
  }
}
```

### Images from fields

An `image` block in a template can name an asset field of the type with `field` in place of `asset`. It shows the asset in that field of the note being shown, so every note of the type shows its own picture in the same place: a cover on its page and on its card.

- An image takes exactly one of `asset` and `field`. The schema rejects one with both, or with neither, with "takes `asset` or `field`, not both" or "needs `asset` or `field`".
- `field` names an [`asset`](#asset) field of the type. The schema rejects any other name with "`"<name>"` names no field of the type", and a field of another kind with "an image shows an asset field, and `"<name>"` is a ... field".
- A field with `multiple = true` shows every image in it, in order, in a row that wraps. `width` applies to each image.
- An asset in the field that is not an image (by its file name: `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.avif` or `.svg`) is skipped. An empty field shows nothing on a card. On the note page it offers to add an image, by choosing a file or dropping one, which uploads the file and sets the field (for a multiple field, adds it). It shows nothing there either when the default template also places the field with a slot or an editable control, which has its own upload.
- `alt` and `caption` may hold `{{name}}` [field references](#field-references), checked like those in `text`.
- `field` is template-only. In a note's own content, an image with `field` fails validation with "is allowed only in a template".

```wcl
template card {
  tab "Card" {
    image {
      field = "cover"
      alt   = "Cover of {{title}}"
    }
    slot "status"
  }
}
```

## Handlers

A `handler` block registers a wscript function to run on an event for notes of the type (D9, D30). The label is the event.

| Event | Runs | Can |
|-------|------|-----|
| `before_create`, `before_update`, `before_delete` | Synchronously in the write path, before the commit | Change the note, or reject the save with a message |
| `after_create`, `after_update`, `after_delete` | From a queue after the commit | Run side effects, call allowlisted HTTP hosts, read named secrets |

| Item | Form | Default | Meaning |
|------|------|---------|---------|
| `script` | string | required | The script name, the stem of `scripts/<script>.ws` |
| `function` | string | the event name | The function in that script to call |
| `allow_domains` | list of strings | `[]` | Host names that HTTP host functions may reach. `["*"]` allows any http or https host. `after_*` handlers only |
| `secrets` | list of strings | `[]` | Secret names the handler may read. `after_*` handlers only |

Several handlers may name the same event. They run in file order, and the first rejection stops the chain.

```wcl
handler "before_update" {
  script   = "specimen"
  function = "count_words"
}

handler "after_create" {
  script        = "specimen"
  function      = "announce"
  allow_domains = ["hooks.example.com"]
  secrets       = ["announce_token"]
}
```

A handler function takes one `Value`, the note as `{ id, type, event, version, note }`, where `note` is `{ fields, blocks }` and `version` is `null` for a note being created. What it returns:

| Event | Return |
|-------|--------|
| `before_create`, `before_update` | A note body, `{ fields, blocks }` or a whole note, replaces the note. A body without `blocks` keeps the note's blocks. `null` or unit leaves the note as it is. `Err(message)` rejects the save with `message` |
| `before_delete` | `Err(message)` rejects the delete. Anything else lets it go ahead |
| `after_*` | Ignored, except that `Err(message)` records the run as failed |

Only a handler writes a `computed` field. A change a handler makes through `vault::create`, `vault::update` or `vault::delete` goes through the write path again and runs that note's handlers. In a `before_*` handler, the change joins the write in progress and commits with it. A chain of handler runs, each set off by the one before, stops at a depth of 8 runs with an error that names the chain and any cycle in it.

A schema change can name a migration: a function, `migrate` by default, of a script in `scripts/`. It takes each note of the type in the same form, after any renames and defaults, and returns the note body to write, or `null` to leave the note. It cannot change other notes.

## Note files

`notes/<type>/<ULID>.wcl` holds one note of type `<type>`. The note's id is the ULID in the file name. A note file holds, in this order:

1. The built-in fields, which every note has:

   | Field | Encoding | Written by | Meaning |
   |-------|----------|------------|---------|
   | `title` | string | the client | Required, not empty. Shown wherever the note is listed or mentioned |
   | `created` | datetime, UTC (`Z`) | the server | When the note was created |
   | `updated` | datetime, UTC (`Z`) | the server | When any write last changed the file, including a reverse-side reference update |
   | `archived` | bool, only ever `true` | the server | Present only on an archived note. A note that is not archived does not hold it |
   | `conflict_of` | string, a note ULID | the device's sync engine | Present only on a sync conflict copy: the id of the note it was copied from. See [Conflict copies](#conflict-copies) |

   Clients cannot set `created` or `updated`. The server ignores either value on a write and writes its own.

   Clients cannot set `archived` by writing the note either. A write keeps the archived state the note already has, whatever the submitted note holds, and a new note is never archived. Only archiving changes it: `POST /api/notes/{id}/archive` and `/unarchive`, `vault archive` and `vault unarchive`, or `vault::archive` and `vault::unarchive` in a script. See [Archived notes](#archived-notes).
2. One field per non-compound field of the type that has a value, in type-file order. An optional field with no value is left out.
3. One block per compound field (`content`, `dict`, `ink`, `canvas`) that has a value, in type-file order, labelled with the field name.

The note's version token is not stored in the file. It lives in `state.db` (D12, D13).

A note may hold a field that its type no longer defines, for example after a field is removed outside a schema change. `store` keeps the data, `schema` flags the note, and the API lists flagged notes (D6). The canonical writer keeps unknown items after the known ones, in their original order.

```wcl
title    = "Ship the importer"
created  = "2026-10-12T09:15:00Z"
updated  = "2026-10-12T10:42:07Z"
estimate = 3
due      = "2026-11-01"
status   = "Active/In progress"
parent   = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB"

content body {
  paragraph {
    text = "Import from **Capacities** first."
  }
}
```

### Conflict copies

When a device and the server both changed a note and their changes overlap, the device's sync engine keeps the server's version and saves the device's as a new note of the same type, a conflict copy (D20, D28). The copy is titled `<title> (conflict, <device name>)` and holds `conflict_of`, the id of the original, after `archived`:

```wcl
title       = "Ship the importer (conflict, Pixel)"
created     = "2026-11-03T16:20:41Z"
updated     = "2026-11-03T16:20:41Z"
conflict_of = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB"
estimate    = 5
```

- **A field of every note.** `conflict_of` is built in, like `title`, so a copy of any type can hold it, whether or not the type has a content field to mention the original in. A type cannot define a field of that name.
- **A link.** The index records it as a link from the copy to the original, so the original's link graph shows its copies. The note page of a copy shows a banner that links to the original.
- **Kept by writes.** A write keeps whatever `conflict_of` the submitted note holds. Removing it makes the copy an ordinary note.

### Archived notes

Archiving takes a note out of the way without deleting it. An archived note holds `archived = true` after `updated`:

```wcl
title    = "Ship the importer"
created  = "2026-10-12T09:15:00Z"
updated  = "2026-11-03T16:20:41Z"
archived = true
estimate = 3
```

- **A normal write.** Archiving and unarchiving are updates at a base version, like any other write (D12). Each records a version of its own in the history, even straight after the same session's autosave, sets `updated`, and runs the type's `before_update` and `after_update` handlers. Archiving a note that is already archived, or unarchiving one that is not, changes nothing and returns the current version.
- **Still a note.** An archived note opens by id, can be edited, and its references and mentions still resolve. A write keeps it archived. Restoring an earlier version keeps the state the note has now, and a note deleted while archived comes back from the trash archived.
- **Left out of listings.** `index.db` keeps the flag in the `archived` column of `notes`, and the full-text index keeps it beside each document. Every query, full-text search and note listing leaves archived notes out unless it asks for them; see [Archived notes in queries](#archived-notes-in-queries). The link graph leaves archived neighbours out.
- **Daily notes.** An archived daily note is still the note for its day: opening the day opens it, and no second note is created.

`vault/notes/specimen/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA.wcl` uses every field kind and every content block kind. `vault/notes/board/01JB8Z4K2MQ7V3X9T5R6W8Y2ZF.wcl` is a note that is mostly one canvas, using both canvas node kinds.

## Content blocks

A content field, and a template tab, hold an ordered sequence of these blocks (D34). The block kinds are closed: `store` rejects any other kind. Items marked required must be written. The text items marked *markup* use the [inline markup](#inline-markup).

### `heading`

| Item | Form | Meaning |
|------|------|---------|
| `level` | int | Required. 1, 2 or 3 |
| `text` | string, markup | Required |

```wcl
heading {
  level = 2
  text  = "Plan"
}
```

### `paragraph`

| Item | Form | Meaning |
|------|------|---------|
| `text` | string, markup | Required. A newline in the text is a line break inside the paragraph |

```wcl
paragraph {
  text = "Import from **Capacities** first, then [[01JB8Z4K2MQ7V3X9T5R6W8Y2ZB|the parent]]."
}
```

### `list`

| Item | Form | Meaning |
|------|------|---------|
| `style` | string | Required. `bullet`, `numbered` or `todo` |
| `item { ... }` | block | At least one |

Each `item` holds:

| Item | Form | Meaning |
|------|------|---------|
| `text` | string, markup | Required |
| `checked` | bool | `todo` lists only. Default `false` |
| `item { ... }` | block | Nested items, which take the list's style |

```wcl
list {
  style = "todo"

  item {
    text    = "Export a sample from Capacities"
    checked = true
  }

  item {
    text = "Map Capacities objects to types"

    item {
      text = "People become `person` notes"
    }
  }
}
```

### `quote`

| Item | Form | Meaning |
|------|------|---------|
| `text` | string, markup | Required |

```wcl
quote {
  text = "Make it work, make it right, make it fast."
}
```

### `image`

| Item | Form | Meaning |
|------|------|---------|
| `asset` | asset reference | An image asset. Required, except in a template, where an image takes `asset` or `field` |
| `field` | string | Template only. An [`asset`](#asset) field of the type: the image shows the asset in that field of the note being shown |
| `alt` | string | Alternative text |
| `caption` | string | Caption shown under the image |
| `width` | int | Display width in CSS pixels. Default: the content width |

```wcl
image {
  asset   = "01JB9A0C3D4E5F6G7H8J9K0M1N"
  alt     = "Importer architecture sketch"
  caption = "The importer reads an export folder."
  width   = 640
}
```

In a template, an image can show a field of the note instead of a fixed asset (see [Images from fields](#images-from-fields)):

```wcl
image {
  field = "cover"
  alt   = "Cover of {{title}}"
}
```

### `asset`

An attached file, shown as a download card with the file name from `assets/<ULID>/<filename>`. A PDF (a file name ending in `.pdf`) is shown in an inline viewer instead, unless `card` is true.

| Item | Form | Meaning |
|------|------|---------|
| `asset` | asset reference | Required |
| `caption` | string | Caption shown with the file |
| `height` | int, 160 to 4000 | Height of the PDF viewer in screen pixels. Default: 560 |
| `card` | bool | Show a PDF as the download card rather than the viewer. Default: false |

```wcl
asset {
  asset   = "01JB9A0C3D4E5F6G7H8J9K0M1P"
  caption = "Sample export"
}
```

### `code`

| Item | Form | Meaning |
|------|------|---------|
| `language` | string | CodeMirror language name or alias for highlighting, in lower case, such as `rust` or `bash`. `stdout` is special: see below. Default: plain text |
| `caption` | string | Caption or file name |
| `source` | string or heredoc | Required. The code, verbatim. Markup is not interpreted |

```wcl
code {
  language = "rust"
  caption  = "Entry point"
  source   = <<'END'
  fn main() {
      println!("importing");
  }
  END
}
```

A shell session is two code blocks: the command as a `bash` code block, then what it printed as a `stdout` code block.

`language = "stdout"` marks program output. It is not a CodeMirror language: the block is shown as plain, unhighlighted text without line numbers, in a muted output style and labelled "Output". Standard error has no separate form; put it in the same `stdout` block.

```wcl
code {
  language = "bash"
  source   = "vault import --dry-run export/"
}

code {
  language = "stdout"
  source   = "42 notes would be created"
}
```

### `mermaid`

| Item | Form | Meaning |
|------|------|---------|
| `source` | string or heredoc | Required. Mermaid source, rendered in the browser (D36) |

```wcl
mermaid {
  source = <<'END'
  graph LR
    export --> importer --> vault
  END
}
```

### `ink`

A drawing inside the content. See [Ink](#ink). An ink content block has no label.

### `query_view`

A live query with saved views. See [Query views](#query-views).

### `canvas`

A canvas inside the content. See [Canvas](#canvas). A canvas content block has no label.

### `columns`

A layout block: two to six columns side by side, each holding its own ordered sequence of content blocks. It takes no items of its own, only `column` blocks.

| Item | Form | Meaning |
|------|------|---------|
| `column { ... }` | block | From 2 to 6, in order from left to right |

Each `column` holds:

| Item | Form | Meaning |
|------|------|---------|
| `width` | int or float | Optional, default `1`. Greater than 0. The column's share of the row: a column's width is its `width` over the sum of the row's widths, so `2` and `1` split the row two thirds to one third |
| any [content block](#content-blocks) but `columns` | block | The column's blocks, in order. A column may be empty |

Rules:

- A column cannot hold another `columns` block: layouts do not nest. `schema` rejects one with "a column cannot hold another columns block".
- A canvas card may hold a `columns` block, and its columns follow the card's rule: no canvas.
- In a [template](#templates) tab, a column may also hold `slot` blocks, which place their fields as a slot in the tab does. A slot in a note's column is rejected.
- Text inside columns is searched and its mentions are links, as anywhere else in the content.
- Where the space a `columns` block has (the pane, a card) is narrower than about 560 pixels, the columns stack, top to bottom in column order. The file does not change.
- The editor writes `width` on every column. A layout left with one column after an edit is not written as a `columns` block: its blocks take its place.

```wcl
columns {
  column {
    width = 2

    heading {
      level = 3
      text  = "Risks"
    }

    paragraph {
      text = "Rate limits on the export API."
    }
  }

  column {
    width = 1

    list {
      style = "bullet"

      item {
        text = "Retry with backoff"
      }
    }
  }
}
```

### `toggle`

A summary line, always shown, that opens and closes the blocks nested under it.

| Item | Form | Meaning |
|------|------|---------|
| `text` | string, markup | Required. The summary. It may be empty |
| `level` | int | Optional, default `0`. `0` shows the summary as plain text; `1`, `2` or `3` shows it as a heading of that level, which counts as a heading wherever the app lists headings |
| any [content block](#content-blocks) but `columns` | block | The nested blocks, in order. A toggle may hold none |

Rules:

- Whether a toggle is open is not stored in the note, so opening one is not an edit, a version or a sync change. Each device remembers it for itself, per note and per block. The web client keys it by the block's place in the content, as `2` or `3.0`, since content blocks have no ids; a block that moves takes the state of the place it moves to. A toggle starts closed.
- A toggle holds no `columns` block: layouts stay at the top of the content, as a column holds none. `schema` rejects one with "a toggle cannot hold a columns block". A toggle in a canvas card follows the card's rule, at any depth: no canvas.
- Text in the summary and in the nested blocks is searched, and its mentions are links, whether the toggle is open or not. Opening a note from a search, with the search's text, opens every toggle whose nested blocks hold that text.

```wcl
toggle {
  level = 2
  text  = "Rollback steps"

  list {
    style = "numbered"

    item {
      text = "Restore the last export"
    }
  }
}
```

### `admonition`

A callout: nested blocks in a coloured box with an icon and a title, whose `kind` sets the colour, the icon and the title shown when it has none.

| Item | Form | Meaning |
|------|------|---------|
| `kind` | string | Required. One of the kinds below |
| `title` | string, markup | Optional. When absent, the kind's label is shown |
| `colour` | string | Optional. A [palette](#inline-markup) colour, in place of the kind's |
| `icon` | string | Optional. An icon name, as a type's `icon` takes, in place of the kind's. `schema` rejects a name the client has no icon for |
| `collapsible` | bool | Optional, default `false`. When `true`, the admonition opens and closes as a toggle does, with the same per-device state, starting open |
| any [content block](#content-blocks) but `columns` | block | The nested blocks, in order. An admonition may hold none |

| `kind` | Colour | Icon | Label |
|--------|--------|------|-------|
| `note` | `gray` | `pencil` | Note |
| `info` | `blue` | `info` | Info |
| `tip` | `green` | `lightbulb` | Tip |
| `success` | `green` | `circle-check` | Success |
| `warning` | `orange` | `triangle-alert` | Warning |
| `danger` | `red` | `octagon-alert` | Danger |
| `question` | `purple` | `circle-question-mark` | Question |
| `quote` | `brown` | `quote` | Quote |

An admonition, like a toggle, holds no `columns` block ("an admonition cannot hold a columns block"), and its title and nested blocks are searched with their mentions as links.

```wcl
admonition {
  kind  = "warning"
  title = "Exports **expire** after a week"

  paragraph {
    text = "Run the import before then."
  }
}

admonition {
  kind        = "tip"
  colour      = "pink"
  icon        = "flame"
  collapsible = true

  paragraph {
    text = "Hidden until the reader opens it."
  }
}
```

### `control`

A form control bound to one field of the note it is shown on: a text box, a slider, a toggle, a calendar and so on. It shows the field's current value and editing it writes the field, so the field stays the only place the value lives. A control, a slot, a `{{name}}` reference and the properties sidebar showing one field always agree, and two controls may bind the same field. Controls are mostly placed in [templates](#templates), to lay a type out as a form, but a note's own content and a canvas card may hold them too.

| Item | Form | Meaning |
|------|------|---------|
| `field` | string | Required. The field the control edits: `title` or a field of the note's type |
| `kind` | string | Required. The control kind, one of the table below |
| `label` | string | Optional. Shown beside the control in place of the field's label |
| `placeholder` | string | Optional. Text shown while the field is empty. Only `text`, `textarea`, `number`, `dropdown` and `chips` take it |
| `min`, `max` | int or float | `number` and `slider` only. The lowest and highest value the control offers; a slider needs both. `min` is below `max` |
| `step` | int or float | `number` and `slider` only. Greater than 0. The amount one step moves the value; default `1` |
| `max` | int | `rating` only. The number of stars, from 1 to 10; default `5` |

The control kinds, and the fields each fits:

| Kind | Shows | Fits |
|------|-------|------|
| `text` | A one-line text box | `string`, `url`, `int`, `float` |
| `textarea` | A text box of several lines | `string` |
| `number` | A number box with step buttons | `int`, `float` |
| `slider` | A slider from `min` to `max` | `int`, `float` |
| `toggle` | An on/off switch | `bool` |
| `checkbox` | A checkbox | `bool` |
| `dropdown` | A list to choose one value from: a taxonomy's terms with their hierarchy, or a note search for a reference | single `select`, `taxonomy` or `reference` |
| `radio` | One radio button per option | single `select` |
| `chips` | The values as chips, removed one at a time, with a box that adds one and suggests values. On an open taxonomy the box can add a new term | `list` of strings or terms, multiple `select`, `taxonomy` or `reference` |
| `calendar` | A month calendar in place. For a range, the first click picks the start and the second the end | `date`, `date_range` of dates |
| `datetime` | A date and time box | `datetime` |
| `time` | The time of day of a datetime. A new time keeps the date the field has, or takes today's when it is empty | `datetime` |
| `rating` | Stars from 1 to `max` | `int` |

Rules:

- `schema` checks a control where it knows the note's type: in a template, against the type the template belongs to, and in a note's content field or canvas field, including inside columns and canvas cards, against the note's type when the note is written. The field must exist, the control must fit its kind and whether it holds one value or several, and the kind-specific items must be valid. Each error names the control block and the reason, such as `content "body" > columns#1 > column#2 > control#1.field: a toggle control fits a bool field, and "due" is a date field`.
- On an `int` field, `min`, `max` and `step` are whole numbers.
- A control may bind a computed field. It shows the value and cannot change it.
- A control cannot clear a required field: the control keeps the value and says why.
- A field bound by a control in the `default` template, or in the note's own content, counts as placed: the properties sidebar does not list it again. A control on a computed field places nothing.
- In a canvas card, a control binds the note that holds the canvas. Where there is no such note, the control is shown disabled.
- A field rename in a type change renames the `field` of every control bound to it, in templates and in notes. A control that names a removed field makes its note invalid, so the type change is refused until the control is removed.

```wcl
columns {
  column {
    width = 1

    control {
      field       = "subtitle"
      kind        = "text"
      label       = "Subtitle"
      placeholder = "One line under the title"
    }

    control {
      field = "score"
      kind  = "slider"
      min   = 0
      max   = 10
      step  = 0.5
    }
  }

  column {
    width = 1

    control {
      field = "due"
      kind  = "calendar"
    }

    control {
      field = "estimate"
      kind  = "rating"
      max   = 5
    }
  }
}
```

## Inline markup

These text items use the inline markup in PRD Appendix A (D41): `heading.text`, `paragraph.text`, `item.text`, `quote.text`, `toggle.text` and `admonition.title`, wherever the block sits, a canvas card included. No other string is parsed as markup.

| Markup | Renders as |
|--------|------------|
| `**text**` | Bold |
| `*text*` | Italic |
| `++text++` | Underline |
| `~~text~~` | Strikethrough |
| `` `text` `` | Inline code |
| `==text==` | Highlight |
| `{red}text{/}` | Text in a palette colour: `gray`, `brown`, `orange`, `yellow`, `green`, `blue`, `purple`, `pink` or `red` |
| `[label](https://example.com)` | Link |
| `[[01JB8Z4K2MQ7V3X9T5R6W8Y2ZB]]` | Mention of a note, shown with its current title |
| `[[01JB8Z4K2MQ7V3X9T5R6W8Y2ZB\|label]]` | Mention of a note, shown with the label |
| `[[+task\|Fix the pump]]` | Placeholder link: a note of type `task` titled `Fix the pump` that does not exist yet |
| `\*` | A literal `*`. A backslash escapes any markup character |

Inside a template's fixed blocks, `{{name}}` also renders a field's value; see [Field references](#field-references). Everywhere else it is plain text.

A mention holds a ULID, and `index` records it like a reference, so `linked_from` and backlinks see it. Inside a WCL string a literal backslash is written `\\`, so the markup escape `\*` appears in the file as `\\*`.

A placeholder link is `[[+<type>|<title>]]`. `<type>` is a type name and must name a type that exists; a note or template whose text names any other type fails validation, naming the text item. `<title>` runs to the first unescaped `]]` and is not blank. A backslash escapes `|`, `]` and `\` in it, as in a mention's label. The `+` keeps a placeholder apart from a mention, since a ULID never starts with `+`. Text that does not match this form, such as `[[+task]]`, is plain text. Deleting a type does not rewrite the placeholders that name it: a note holding one fails validation until the placeholder is removed or changed.

A placeholder has no target, so `index` records no link for it; its title is searched like the text around it. In the app, clicking one creates a note of `<type>` titled `<title>`, then rewrites the placeholder in place as a mention `[[<new ULID>]]` through a normal versioned save of the note that holds it. In a template's fixed blocks and in a version shown from history, a placeholder only shows.

```wcl
paragraph {
  text = "Mark risks {red}in red{/}, ==highlight== questions, and see [[01JB8Z4K2MQ7V3X9T5R6W8Y2ZB]]. Then [[+task|Fix the pump]]."
}
```

## Query views

A `query_view` block runs one query and shows its results through one or more saved views (D8, D33).

| Item | Form | Meaning |
|------|------|---------|
| `title` | string | Heading shown above the view |
| `query` | string | Required. A query in the grammar of PRD Appendix B, with the `include archived` clause of [Archived notes in queries](#archived-notes-in-queries). `this` is the note that holds the block |
| `default_view` | string | The view shown first. Default: the first view |
| `view <name> { ... }` | block | At least one. `<name>` is unique within the block |

In the query's `search "<text>"` clause every word must appear in the note's title or text, ignoring case. Earlier words match whole words, and the last word also matches as a prefix: `search "meet"` finds `meeting`. A trailing space, as in `search "meet "`, or a last word of one character, makes the last word whole. A prefix stands for at most the 50 most frequent words it starts.

Each `view` holds a `kind` and the items that kind uses:

| Item | Form | Meaning |
|------|------|---------|
| `kind` | string | Required. One of the kinds below |
| `title` | string | Tab label for the view. Default: the view name |
| `template` | string | The template used to render each note: `default` or `card` |
| `columns` | list of strings | `table`: the fields shown as columns, in order. Default: `["title"]` |
| `group` | string | `kanban`: required, a single-term `taxonomy` field. Its terms are the columns |
| `date` | string | `calendar`, `timeline`: required, a `date`, `datetime` or `date_range` field |
| `mode` | string | `calendar`: `month`, `week` or `day`. Default `month` |
| `location` | string | `map`: required, a `location` field. An image-map field shows its image |
| `place "<ULID>" { ... }` | block | `canvas`: a saved position for one result note |

| View kind | Shows | Default `template` |
|-----------|-------|--------------------|
| `table` | Rows and columns | not used |
| `cards` | A grid of cards | `card` |
| `list` | One line per note, the title | not used |
| `details` | Each note expanded | `default` |
| `kanban` | One column per term of `group`. Dropping a card writes that field | `card` |
| `calendar` | Notes on the date of `date` | not used |
| `timeline` | Notes as points or spans along `date` | not used |
| `map` | A pin per note at `location` | `card`, in the pin's popup |
| `canvas` | Results as cards on a canvas | `card` |

A `place` holds:

| Item | Form | Meaning |
|------|------|---------|
| label | note ULID | The result note this position belongs to |
| `x`, `y` | float | Required. Top-left corner in canvas units |
| `width`, `height` | float | Card size. Default: the template's natural size |

A result with no `place` is laid out automatically after the placed ones. A `place` for a note that is no longer in the results is kept and ignored.

```wcl
query_view {
  title        = "Open children"
  query        = "from specimen where parent == this && !done sort due asc"
  default_view = "board"

  view table {
    kind    = "table"
    columns = ["title", "status", "due"]
  }

  view board {
    kind  = "kanban"
    group = "status"
  }

  view calendar {
    kind = "calendar"
    date = "due"
  }

  view canvas {
    kind = "canvas"

    place "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC" {
      x = 0.0
      y = 0.0
    }
  }
}
```

### Archived notes in queries

A query leaves archived notes out unless it asks for them. It asks in one of two ways:

| Query | Reads |
|-------|-------|
| `from task` | Notes that are not archived |
| `from task where archived` | Archived notes only |
| `from task where !archived` | Notes that are not archived, as with no condition |
| `from task where archived \|\| done` | Any condition that names `archived` decides for itself |
| `from task include archived` | Every note, archived or not |

`archived` is a built-in bool field of every note: `true` for an archived note and `false` for any other. A `where` expression that names it, outside a closure that binds the name, takes over from the default. Otherwise the query reads as if it ended its `where` with `&& !archived`.

`include archived` is a clause that follows the type list: `from <types> include archived where ...`. The full clause order is `from`, `include archived`, `where`, `search`, `sort`, `group by`, `select`, `limit`. `include` is not reserved: it only has this meaning straight after the type list, so a field may still be called `include`.

The API's `/api/query` takes an optional `archived` of `include` or `only`, which adds `include archived` or `&& archived` to the query. `/api/search` and `/api/notes` take the same parameter, with `exclude` as the default.

## Ink

An ink drawing is an `ink` block (D18, D29). It is a content block with no label, or a note's `ink` field with the field name as its label.

A drawing is an endless plane. Its block on the page is a view onto the plane, as wide as the column it is in and `height` pixels tall. Strokes can lie anywhere on the plane, at negative or large coordinates.

| Item | Form | Meaning |
|------|------|---------|
| `width` | int | Optional. The width of the page a drawing made before the plane was endless had, in ink units. A drawing with a `width` and no `viewport` opens at the origin with that width fitted to the column, no larger than 100%, as it showed before. The app keeps the item and does not write it on new drawings |
| `height` | int, 1 to 4000 | The height of the view on the page, in CSS pixels. Default 400 |
| `background` | string | `blank`, `lines`, `grid` or `dots`. Default `blank`. The pattern covers the whole plane and moves with it |
| `viewport { ... }` | block | Optional, at most one, ahead of the strokes. The view saved with the drawing. Without it a drawing opens at the origin at 100%, or as `width` says |
| `stroke { ... }` | block | The strokes, in drawing order. Later strokes draw on top |

The `viewport` holds:

| Item | Form | Meaning |
|------|------|---------|
| `x`, `y` | float | Required. The ink point at the top-left corner of the view |
| `zoom` | float | Required. Greater than 0: screen pixels per ink unit, so `1.0` is 100%. The app keeps it from 0.1 to 8 and writes it to three decimal places, and `x` and `y` to two |

Each `stroke` holds:

| Item | Form | Meaning |
|------|------|---------|
| `tool` | string | Required. `pen`, or `highlighter`, which draws translucent, behind pen strokes and at one width whatever the pressure |
| `colour` | ink colour | Required. `#rrggbb`, or `ink` for the default ink. See [Ink colours](#ink-colours) |
| `size` | float | Required. The perfect-freehand base size in ink units |
| `points` | list of floats | Required. Flat triples `x, y, pressure`. `x` and `y` are ink units on the plane, which are CSS pixels at 100% zoom, and may be negative. `pressure` is 0.0 to 1.0, and 0.5 where the device reports none. At least one triple |

The renderer feeds each stroke's points to perfect-freehand, and draws only the strokes in or near the view. Erasing deletes whole strokes, so no eraser strokes are stored. The writer rounds coordinates to two decimal places and pressure to three.

In the app a stylus draws with the chosen tool, and its eraser end or barrel button erases. The mouse and one finger pan, two fingers pan and pinch to zoom, the wheel pans, and Ctrl or Cmd with the wheel, or a trackpad pinch, zooms. **Draw with mouse and touch**, a toggle kept per device, makes the mouse and one finger draw instead; two fingers still pan and zoom. While a stylus touches the drawing, fingers are ignored. The pen starts in the default ink at size 4, with sizes 2, 4, 8 and 16; the highlighter starts yellow (`#facc15`) at size 24, with sizes 16, 24, 36 and 48, and each tool keeps its own colour and size. The view is saved into `viewport` a moment after it stops moving, through the normal autosave, so a pan is not an undo step and a run of pans is one version in the history. A read-only drawing, such as a version shown from history, shows its saved view and cannot be panned.

The handle on the bottom edge of an editable drawing sets `height`, from 80 to 4000, and a double-click on it puts back 400. Resizing changes only how much of the plane shows: no stroke moves or scales, and the view keeps its top-left corner. The app writes `height` on every drawing it saves; a file without one reads as 400.

### Ink colours

The paper follows the app's theme: white in the light theme, a dark panel colour in the dark theme, with its lines, grid or dots in a faint shade of the text colour. A stroke's `colour` is stored once and draws so it reads on either paper, so a drawing made on a device in one theme reads on a device in the other:

| Stored `colour` | Light theme | Dark theme |
|-----------------|-------------|------------|
| `ink`, the default ink | Dark ink, `#1f2937` | Light ink, `#e2e8f0` |
| `#1f2937` on a pen stroke, the default ink as drawings stored it before `ink` | As `ink` | As `ink` |
| A palette hue as the pen stores it: `#6b7280`, `#92400e`, `#ea580c`, `#ca8a04`, `#16a34a`, `#2563eb`, `#9333ea`, `#db2777`, `#dc2626` | As stored | The hue's dark-theme shade, as inline palette colours take |
| Any other pen colour | As stored | As stored |
| A highlighter colour | As stored | As stored, fainter |

A pen colour of the last two kinds with a contrast ratio below 2 against the paper, such as black on dark paper or white on light paper, draws with its lightness turned over and its hue kept. The app never rewrites a stored colour: the pen's **Ink** swatch writes `ink`, and the toolbar's swatches show each colour as it draws in the current theme.

```wcl
ink {
  height     = 300
  background = "lines"

  viewport {
    x    = -40.0
    y    = -20.0
    zoom = 1.0
  }

  stroke {
    tool   = "pen"
    colour = "ink"
    size   = 4.0
    points = [10.0, 20.0, 0.5, 14.5, 22.0, 0.6, 20.0, 25.5, 0.7]
  }

  stroke {
    tool   = "highlighter"
    colour = "#facc15"
    size   = 24.0
    points = [-120.0, 60.0, 0.5, 240.0, 60.0, 0.5]
  }
}
```

## Canvas

A canvas is TheVault's own WCL format, not JSON Canvas (D15, D26). It is a `canvas` block: a content block with no label, or a note's `canvas` field with the field name as its label.

| Item | Form | Meaning |
|------|------|---------|
| `height` | integer | The height of the canvas on the page, in screen pixels, from 160 to 4000. Default 480. Its width always fills the page or pane it is in. The handle on the canvas's bottom edge sets it, and a double-click on the handle puts back the default, which leaves the item out. Resizing changes only how much of the canvas shows: the viewport and the nodes stay where they are |
| `viewport { ... }` | block | Required. The view saved with the canvas |
| `group <id> { ... }` | block | Labelled frames that hold nodes |
| `node <id> { ... }` | block | Items on the canvas, in z-order. Later nodes draw on top |
| `edge <id> { ... }` | block | Connections between nodes |

`<id>` is an identifier, unique across the groups, nodes and edges of one canvas. The app names a new item after its kind, or an edge after the nodes it joins, then adds an underscore and five random lower-case letters and digits, as in `card_k3f9x`, `group_0m2ad` or `card_k3f9x_to_note_7tq4z_p1c8w`. The suffix keeps ids made on two devices offline from clashing when the canvases sync (D30). An item keeps the id it was given, so ids made before the suffix existed stay as they are. Coordinates are canvas units, floats, with `y` increasing downwards. A canvas in a template's fixed content shows at the template's `height` on every note.

### `viewport`

| Item | Form | Meaning |
|------|------|---------|
| `x`, `y` | float | Required. The canvas point at the top-left of the view |
| `zoom` | float | Required. Scale factor, greater than 0. `1.0` is 100% |

### `group`

Groups draw behind every node.

| Item | Form | Meaning |
|------|------|---------|
| `label` | string | Shown on the frame |
| `x`, `y`, `width`, `height` | float | Required. The frame |
| `colour` | palette colour | Frame colour |

A node belongs to a group when it names the group in its `group` item. Moving a group moves its nodes.

### `node`

Every node takes these items:

| Item | Form | Meaning |
|------|------|---------|
| `kind` | string | Required. `card` or `note` |
| `x`, `y`, `width`, `height` | float | Required. The node's box |
| `group` | string | The id of the group the node belongs to |
| `colour` | palette colour | Node colour |

Each node kind adds its own items:

| Node kind | Items | Shows |
|-----------|-------|-------|
| `card` | Content blocks, as in [Content blocks](#content-blocks): headings, paragraphs, lists, quotes, images, files, code, diagrams, drawings, query views, column layouts and controls. Not a `canvas`, in the card or in its columns | The blocks, in order, as a content field shows them. A query view in a card shows live results; its `this` is the note that holds the canvas |
| `note` | `note`: note ULID, required. `template`: default `card`. No blocks | A note rendered with a template |

#### Reading older canvases

Canvases written before cards had six node kinds: `text`, `note`, `query`, `asset`, `link` and `ink`. TheVault still reads them, and reads each node of the older kinds as the card that shows the same thing. The node keeps its id, box, group and colour, and edges are unchanged:

| Older kind | Read as a card holding |
|------------|------------------------|
| `text` | One `paragraph` per run of lines between blank lines, with its inline markup |
| `asset` | An `image` block naming the asset |
| `link` | A `paragraph` holding `[url](url)` |
| `query` | A `query_view` with the node's `query` and one view of the node's `view` kind (`cards` by default). A `cards` view keeps the node's `template` |
| `ink` | An `ink` block the size of the node, holding the node's strokes |

The file keeps the older form until its note is next saved, which writes the cards. A note sent to the server with nodes of the older kinds, in WCL or JSON, is converted the same way before it is checked and written. `vault reindex` reports no error for the older kinds, and TheVault never writes them. A type file is stored as it is given, so a template canvas in the older form stays in that form, read as cards, until the template is next saved.

### `edge`

| Item | Form | Meaning |
|------|------|---------|
| `from`, `to` | string | Required. Node ids in the same canvas. An edge is drawn from `from` to `to` |
| `from_side`, `to_side` | string | `top`, `right`, `bottom` or `left`. Default: the nearest side. The web editor draws edges as right-angled elbows between the closest facing pair of sides, and rewrites both items to that pair whenever an edge is made or either node moves or resizes |
| `label` | string | A caption, drawn at the middle of the edge |
| `colour` | palette colour | Edge colour. The line and its arrowheads take it |
| `arrow` | string | Where arrowheads go: `none`, `start` (at the `from` node), `end` (at the `to` node) or `both`. Default `end` |

```wcl
canvas board {
  height = 640

  viewport {
    x    = -120.0
    y    = -80.0
    zoom = 0.75
  }

  group plan {
    label  = "Plan"
    x      = -40.0
    y      = -40.0
    width  = 1000.0
    height = 480.0
    colour = "blue"
  }

  node goal {
    kind   = "card"
    x      = 0.0
    y      = 0.0
    width  = 260.0
    height = 120.0
    group  = "plan"

    heading {
      level = 2
      text  = "Goal"
    }

    paragraph {
      text = "**Import Capacities**"
    }
  }

  node open_work {
    kind   = "card"
    x      = 340.0
    y      = 0.0
    width  = 240.0
    height = 400.0
    group  = "plan"

    query_view {
      query = "from specimen where under(status, \"Active\") sort due asc limit 10"

      view cards {
        kind = "cards"
      }
    }
  }

  edge goal_to_work {
    from  = "goal"
    to    = "open_work"
    label = "tracks"
    arrow = "end"
  }
}
```

`vault/notes/board/01JB8Z4K2MQ7V3X9T5R6W8Y2ZF.wcl` uses both node kinds, cards holding several kinds of block, and every arrow value.

## Taxonomy files

`taxonomies/<taxonomy>.wcl` defines one global, hierarchical taxonomy (D7). Terms nest to any depth.

| Item | Form | Meaning |
|------|------|---------|
| `label` | string | Required. Display name |
| `description` | string | Help text |
| `allow_new` | bool | Default `false`. `true` makes the taxonomy open: notes may add terms to it. See [Open taxonomies](#open-taxonomies) |
| `term "<name>" { ... }` | block | At least one top-level term, unless `allow_new` is `true`. Order is display order, which is also kanban column order |

Each `term` holds:

| Item | Form | Meaning |
|------|------|---------|
| label | string | Required. The term name. Not empty, contains no `/`, unique among its siblings |
| `colour` | palette colour | Chip, badge and kanban column colour. One of `gray`, `brown`, `orange`, `yellow`, `green`, `blue`, `purple`, `pink` and `red` (OQ-S4); any other value is a validation error |
| `description` | string | Help text |
| `term "<name>" { ... }` | block | Child terms |

A term's path is its name and its ancestors' names, root first, joined by `/`. Notes store paths, and `under(field, "Active")` matches `Active` and every term below it.

A term without a `colour` takes the colour of its nearest ancestor that has one. A term with no coloured ancestor has no colour, and the client colours its chip from a hash of its path. The file holds only the colours written in it: the API returns each term's own `colour`, or `null`, and the client resolves inheritance. In the example below, `Active/In progress` is blue.

```wcl
label       = "Status"
description = "Where a piece of work is."

term "Inbox" {
  colour = "gray"
}

term "Active" {
  colour = "blue"

  term "In progress"

  term "Blocked" {
    colour = "red"
  }
}

term "Done" {
  colour = "green"
}
```

### Open taxonomies

A taxonomy with `allow_new = true` is open. When a note is saved, through the API, the CLI or the web client, with a term path an open taxonomy does not hold, the server adds the term to `taxonomies/<taxonomy>.wcl` in the same commit as the note. This holds for every `taxonomy` field and list of terms on the taxonomy. A closed taxonomy, the default, refuses a path it does not hold, as validation always has.

- Each name in the path is matched to an existing term exactly, or else without case. A note saved with `["gardening"]` when `Gardening` exists holds `Gardening`, and no second term is added.
- Only the last name in the path may be new. It is added as the last child of its parent, with no colour, so it takes its parent's colour. `Gardening/Roses` adds `Roses` under `Gardening`. A path whose parent does not exist, such as `Nowhere/Roses`, is a validation error at the field, and nothing is written. A write may add a term and a term under it together.
- A name may not be empty or contain `/`.
- A write that fails, for any reason, adds no term.
- A planned type change that makes a field a list of terms of an open taxonomy adds the values its notes hold, matched exactly, in the same commit as the type file.
- A field's `default` must still name terms the taxonomy has.

The web client offers a typed name that matches no term as an "Add" suggestion in a multi-term field's chip picker. Single-term fields keep their dropdown and add no terms there.

```wcl
label       = "Tags"
description = "Free-form tags."
allow_new   = true
```

## Task files

`tasks/<task>.wcl` defines one scheduled task (D10). Secret values are never in the file. A task names the secrets it may read, and their values live encrypted in `state.db`.

| Item | Form | Default | Meaning |
|------|------|---------|---------|
| `label` | string | the task name | Display name |
| `description` | string | none | Help text |
| `schedule` | string | required | A five-field cron expression: minute, hour, day of month, month, day of week |
| `timezone` | string | `"UTC"` | IANA time zone the schedule is read in |
| `script` | string | required | The script name, the stem of `scripts/<script>.ws` |
| `function` | string | `"run"` | The function in that script to call |
| `allow_domains` | list of strings | `[]` | Host names that HTTP host functions may reach. `["*"]` allows any http or https host, including hosts on the local network |
| `secrets` | list of strings | `[]` | Secret names the task may read |
| `read_paths` | list of strings | `[]` | Absolute directories the `fs` host functions may read |
| `write_paths` | list of strings | `[]` | Absolute directories the `fs` host functions may read and write |
| `retries` | int | `3` | Retries after a failed run. `0` disables retries |
| `retry_delay` | duration | `"PT1M"` | Delay before the first retry. Each later retry doubles it |
| `timeout` | duration | `"PT30S"` | Wall-clock limit for one run |
| `paused` | bool | `false` | A paused task never fires on its schedule. It can still be triggered by hand |

```wcl
label         = "Daily digest"
schedule      = "0 7 * * 1-5"
timezone      = "Australia/Melbourne"
script        = "digest"
allow_domains = ["hooks.example.com"]
secrets       = ["digest_webhook"]
retries       = 3
retry_delay   = "PT1M"
```

A task's script runs with the `vault`, `http`, `secrets` and `fs` modules. `docs/scripts.md` lists every host function. The script's note changes go through the write path like an API write, are recorded in history as made by `task <task>`, and run the notes' handlers.

`read_paths` and `write_paths` hold absolute paths. A path inside the vault folder is refused when the task is saved, and the `fs` functions refuse the vault at every call, after resolving symbolic links: scripts change notes through `vault`, never by editing vault files.

```wcl
label         = "Weather log"
schedule      = "0 * * * *"
script        = "weather"
allow_domains = ["*"]
read_paths    = ["/srv/inbox"]
write_paths   = ["/srv/exports"]
```

## Device files

`devices/<platform>.wcl` says what a paired device of the platform does with its device events, and which commands it accepts from server scripts (D5, D22). The platform is `linux` or `android`. Device events happen on the device and their handlers run there; note events and scheduled tasks stay on the server. A platform with no file runs no device handlers and accepts no commands.

The file holds `handler` and `command` blocks and nothing else.

| Item | Form | Meaning |
|------|------|---------|
| `handler "<event>" { ... }` | block | A script function to run on a device event. Several handlers may name the same event; they run in file order |
| `command "<name>" { ... }` | block | A command the device accepts, and the script function that runs it. Each command appears at most once |

The device events are a closed list: `sms_received`, `sms_sent`, `mms_received`, `rcs_received`, `call_ended`, `contact_changed`, `contact_deleted` and `share_received`. The commands are a closed list too, the ones the device app implements: `send_sms`, `upsert_contact`, `delete_contact`, `show_notification` and `import_history`. Any other label is a validation error.

Each `handler` and `command` block holds:

| Item | Form | Default | Meaning |
|------|------|---------|---------|
| `script` | string | required | The script name, the stem of `scripts/<script>.ws`. The script must exist when the file is saved |
| `function` | string | the event or command name | The function in that script to call |

```wcl
handler "sms_received" {
  script = "device_messages"
}

handler "call_ended" {
  script = "device_calls"
}

handler "contact_changed" {
  script   = "device_contacts"
  function = "changed"
}

command "send_sms" {
  script = "device_messages"
}

command "show_notification" {
  script = "device_notify"
}
```

A server script queues a command with `vault::device_command`. The server refuses a command the device's platform file does not declare, and one the user has not enabled for that device in Settings > Devices. So declaring a command makes it available, and enabling it is a choice made per device. `docs/scripts.md` gives the value each handler and command function takes, the payload of every event and command, and what a command reports back.

A script that a device file names cannot be deleted. Device scripts reach the `vault` module and the `device` module, and no `http`, `secrets` or `fs`. The home server checks that they compile as device scripts, and never runs them.

A vault with no `devices/android.wcl` runs the built-in default on an Android device. The default handles every device event with the default device scripts and declares every command:

| Script | Handles |
|--------|---------|
| `device_messages` | `sms_received`, `sms_sent`, `mms_received` and `rcs_received`: one `message` note per message, in the `conversation` note of its thread, linked to the `person` holding the number, or for RCS the person of the sender's name. An unknown number creates no person. The commands `send_sms` and `show_notification` |
| `device_calls` | `call_ended`: one `call` note per call, linked to the `person` holding the number |
| `device_contacts` | `contact_changed` and `contact_deleted`: phone contacts and `person` notes kept in step, linked through `device_contacts`; the side changed most recently wins, and a delete only unlinks. The commands `upsert_contact` and `delete_contact` |
| `device_share` | `share_received`: a fleeting note with the shared text, link, images and files |
| `device_import` | The command `import_history`: answers the import to run, by default the command's payload. The app then reads the phone's history and runs each item that has no note yet as its device event |

Each script captures an item once, by its `device_ref`, archived notes included. A device file in the vault names the scripts it runs; a script it names that the vault does not hold runs from the built-in defaults. The server writes the default device file and scripts into a vault that lacks them on every start, and `vault init --starter` writes them too, so they can be edited. [Shipped defaults and upgrades](#shipped-defaults-and-upgrades) says when a newer shipped version replaces one.

## Shipped defaults and upgrades

The app ships defaults into every vault: the built-in types (and the `person` type the phone data types reference), the built-in taxonomy `gmail_label`, the default device file `devices/android.wcl`, the five default device scripts `scripts/device_*.ws`, and the Google sync's paused tasks `tasks/google_calendar.wcl` and `tasks/google_gmail.wcl` with their scripts `scripts/google_calendar.ws` and `scripts/google_gmail.ws`, and the `event` type's write-back script `scripts/google_event.ws` (google-sync D19). On every start the home server brings them up to date, and a device's local vault does the same when the app opens it. The code is `crates/writer/src/shipped.rs`.

1. **Upgrade steps.** One-time upgrades for changes that are not additive, such as a field rename with a migration script. Each has an id, runs once per vault in the order the app lists them, and is recorded in `state.db` once it succeeds. A step that fails stops the server from starting and runs again on the next start. A step that cannot run yet, such as one that needs the secrets key on a server started without its key file, is logged and not recorded: it runs again on the next start, and the server starts. Steps run first, so the seeding below never adds a field under the name a step is about to rename it to. Only the home server runs them; a device takes their results through sync. The steps are:

   | Step | Does |
   |------|------|
   | `0001_person_emails` | A `person` type with `addresses` or `email` and no `emails`: `addresses` becomes `emails`, `email` moves into it and goes (see [Built-in types](#built-in-types)). Its note versions are authored by `task server` |
   | `0002_person_labels` | A `person` type whose `emails`, `phones` and `addresses` have no `labels`: each gains its `labels`, `photo` gains `accept = "image"` when every photo is an image, and the label lists' slots leave the `default` template (see [Built-in types](#built-in-types)). It edits no note |
   | `0003_event_google_handler` | An `event` type with no handler naming `google_event`, as main wrote it before the Google Calendar write-back: it gains the shipped type's three `handler` blocks, before its first template (see [Built-in types](#built-in-types)). The seeding below only adds fields, so it would not carry them. It edits no note |
   | `0004_remove_google_sync` | The Google sync's settings move from the 0.1.7 `google_sync` note type into secrets (google-sync D21). Every `event` handler naming `google_event` gains the `google_settings` secret, as a planned type change that edits no note. When the vault has a `google_sync` note, the first one's settings, cursors and `account` move into `google_settings`, `google_gmail_state`, `google_calendar_state` and `google_account`, each key only where the secret does not hold it already; `calendar_sync_tokens` keys such as `c_me_40example_2ecom` become calendar ids such as `me@example.com`. Then every `google_sync` note is deleted for good, not to the trash, and the type is deleted. A vault with the type and no note just loses the type. Moving a note needs the secrets key: without it the step changes nothing more, keeps the type and the note, and runs again on the next start |
2. **Built-in taxonomies and types.** A built-in taxonomy the vault lacks is added whole, before the types that use it; one the vault has is never changed. A type the vault lacks is added whole. A built-in type the vault has, and its `person` type, gain every field the shipped default declares and the vault's type lacks. The fields are added as a planned schema change that edits no note: a field the vault has is never removed, renamed or changed, even when its kind differs from the shipped one, and no note is rewritten. Each added field goes after the nearest field the shipped default puts before it. When the shipped `default` template places an added field, the vault's `default` template gains that slot, after the nearest slot the shipped template puts before it, else before the nearest slot it puts after it, or at the end of its first tab. Slots inside `columns` and `column` count, so a slot the shipped template puts after `from` in a column lands after `from` in the vault's column. No other template, and nothing else in the `default` template, changes: a vault from before 0.1.13 gains `processed` on `email` and `message`, and its slot on their pages, but not on their cards, which the user can add with `slot "processed"` in `template card`. A field that the user removes from a built-in type is added again on the next start; hide a field with `hidden = true` instead.
3. **Missing files.** Each default device script, the default device file and each shipped task and its script the vault lacks is written, the scripts before the device file or task that names them. A deleted one comes back on the next start; pause a shipped task to stop it. That changes nothing on a device: a vault with no `devices/android.wcl` runs the same built-in default, and a script the device file names cannot be deleted while it does.
4. **Newer shipped files.** `state.db` records the content hash of each shipped device file, task or script the server writes. The app carries the hash of every version of each that it has ever shipped. On start, a file whose content is still a version the app shipped, or still what the server wrote, is replaced by the current shipped version, so a vault that skipped releases is upgraded too. A file the user changed is left alone. When a newer shipped version exists than the one the user changed, the server adds one notification under the bell that names the file and says a newer version exists. It adds no other for that version, on any later start. A file the user changed with no record of what the server wrote, as in a vault from before these records, counts as based on an older version. A device's local vault never adds the notification, since the home server does.

   A task file's top-level `paused` field is the user's pause switch, set by Connect and Disconnect in Settings > Integrations and by pausing a task in Settings > Scheduled, so the comparison ignores it (google-sync D19). The hash of a task file, both the shipped versions' hashes and the one `state.db` records, is taken over its canonical form with `paused` removed. Pausing or unpausing a shipped task therefore leaves it unedited: the next start changes nothing and adds no notification, and a newer shipped version still replaces it. The replacement takes the vault file's `paused` value in place of the shipped one, or has no `paused` when the vault file has none, so an upgrade never pauses or unpauses a task. Any other change to a task file, such as its `schedule`, `timeout` or `allow_domains`, is an edit, and the file is kept as above.

Steps 2 to 4 make one commit. A start that finds nothing to do makes no commit, no change-feed entry and no note version.

## Validating vault files

A vault file is checked in three layers. Each catches what the one before cannot.

1. **Data-only.** `store` parses the file and rejects anything outside [the data-only subset](#the-data-only-subset). `docs/vault-format/data-only.mjs` is a reference implementation of this check.
2. **Structure.** `wcl_lang` validates the file against the WCL schema for its file kind. The schemas describe this version's vocabulary, which is why they are a check for the example files and not a reader's rule. This catches unknown fields and blocks, wrong value types, missing block items, out-of-range heading levels, and canvas edges that name no node.
3. **Semantics.** `schema` checks what WCL cannot express. An attribute of a type or a field that it does not know is a warning, not an error (see [Attributes a reader does not know](#attributes-a-reader-does-not-know)). That covers required top-level fields, the encodings in [Value encodings](#value-encodings), and closed vocabularies such as kinds, styles and colours. It also checks that references and terms exist, and the cross-file rules: inverse fields, template slots and handler events, and the events and commands a device file names.

### The schema files

`docs/vault-format/schema/` holds the structural schemas. Vault files carry no import, so a host puts the schema in front of the file: it registers the schema types in a `wcl_lang::Environment`, or it concatenates the schema source and the file.

| File | Declares |
|------|----------|
| `common.wcl` | Date range and location records, every content block, ink, query view, canvas, `columns`, `control`, `toggle`, `admonition`, `slot`, `content` and `dict`. No `@document` |
| `type.wcl` | The `@document` of a type file, with `field`, `template`, `tab` and `handler` |
| `taxonomy.wcl` | The `@document` of a taxonomy file |
| `task.wcl` | The `@document` of a task file |
| `device.wcl` | The `@document` of a device file, with `handler` and `command` |
| `notes/<type>.wcl` | A generated `@document` for the notes of one type |

### Generated note schemas

A note's top-level fields depend on its type, so its `@document` is generated from `types/<type>.wcl` by these rules. `schema/notes/specimen.wcl` and `schema/notes/board.wcl` are generated from the example types.

- The document type is named after the type in PascalCase with a `Note` suffix, for example `SpecimenNote`.
- `title`, `created` and `updated` are `utf8`. `archived` is `bool?`.
- Each non-compound field is declared under its own name, in type-file order, with the WCL type below. It is optional (`?`) unless `required = true`. If `multiple = true`, the field is `list<T>?` instead.

  | Field kind | WCL type |
  |------------|----------|
  | `string`, `date`, `datetime`, `url`, `select`, `taxonomy`, `reference`, `asset` | `utf8` |
  | `int` | `i64` |
  | `float` | `f64` |
  | `bool` | `bool` |
  | `date_range` | `DateRange` |
  | `location` with `map = "geo"` | `GeoPoint` |
  | `location` with `map = "image"` | `ImagePoint` |
  | `list` | `list<T>`, where `T` is the WCL type of `items` |

- For each compound kind the type uses, add one gather field: `@children("content", max = 1) contents: list<Content>`. The same pattern covers `dict` (`Dict`), `ink` (`Ink`) and `canvas` (`Canvas`). If the type has two fields of one compound kind, raise `max` to the number of those fields.

### Running the checks

```bash
./dev cargo install --git https://github.com/wiltaylor/wcl.git \
  --rev 0531228286cb19d5b73daf7de23e79790a5e15d4 --root "$PWD/.scratch/wcl" wcl
./dev env WCL="$PWD/.scratch/wcl/bin/wcl" docs/vault-format/check.sh
```

`check.sh` checks every file under `docs/vault-format/vault/` against its schema and against the data-only subset. It then confirms that both checks reject the files in `docs/vault-format/invalid/`. It exits non-zero on any failure.

### WCL behaviour this format works around

These hold at the pinned WCL revision `0531228`. Anyone building `store` or `schema` needs to know them.

- `wcl check -` reads stdin and skips schema validation. It prints `OK` for any document that parses. Check a file on disk, or call `schema_diagnostics()` on a document opened with the schema in scope.
- WCL does not report a missing non-optional field on its own. A block enforces required items only through `@block(..., required_fields = [...])`, and a `@document` has no equivalent, so `schema` checks required top-level fields.
- `@schemaless` on a top-level `@document` field is ignored. It works on a block type and on a field inside a block. This is why `dict` is a block rather than a record value.
- Record keys must be identifiers. `{ "Content-Type": "x" }` does not parse, so dict keys are identifiers.
- `extends` does not carry `@children` declarations to the child type, so `content` and `tab` each list every block kind.
- A record value is not checked member by member: `{ start: "x" }` passes as a `DateRange`. `schema` checks record members.
- Each `@children` list gathers one block kind, so mixed content blocks are split across several lists. `store` reads block order from the document's block sequence or the syntax tree, never from the gathered lists.
