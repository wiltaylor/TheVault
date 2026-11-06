---
name: vault-notes
description: Write TheVault notes and types correctly. Use before propose_create, propose_edit or propose_fields when you are not sure of a field's value form or a content block's JSON, after a note or a type was refused, and before propose_type_change for reference, inverse, taxonomy or computed fields and query views (kanban, calendar, map).
---

# Writing notes and types

Every example in this skill is checked: the type files are written to a test vault, and each
note is validated as a create. The person `01JB8Z4K2MQ7V3X9T5R6W8Y2ZB`, the project
`01JB8Z4K2MQ7V3X9T5R6W8Y2ZC` and the image asset `01JB9A0C3D4E5F6G7H8J9K0M1N` stand for notes and
files that exist. In a real vault, take ids from `query_notes`, `search_notes` and `find_assets`,
never from here. `value_format` and `block_format` give the full rules.

## A note as JSON

```
{ "fields": { "<field>": <value>, ... }, "blocks": [ <block>, ... ] }
```

- Plain fields go under `fields`, by the field's `name`. Leave out an optional field that has
  no value.
- A compound field, `content` (the body), `dict`, `ink` or `canvas`, is a block under `blocks`:
  `{ "kind": "content", "labels": ["body"], "blocks": [ ... ] }`, with the field name in `labels`.
- Every block is `{ "kind", "labels", "fields", "blocks" }`, each key optional but `kind`.
- `created`, `updated` and `archived` belong to the server. It ignores them in a write.
- A field `get_type` shows as `computed` is written by the type's handlers only. Never set or
  change it: a write that changes it is refused. Leave it out of a new note.

## Change only what you mean to change

- To set a few fields, such as ticking `processed`, moving a status or a due date, use
  `propose_fields` with the note's `id`, the `base_version` from `get_note`, and only those fields:
  every other field and the whole body stay as they are. `null` removes a field.

```json
{ "processed": true, "stage": "Done", "due": null }
```

- To change the body, `propose_edit` with the whole note as `get_note` gave it, changed where you
  mean: every field and block you leave out is removed.
- A refusal for a conflict means the note changed since you read it: `get_note` again and redo
  the change on the new version.

## The types these examples use

A taxonomy holds the terms of a `taxonomy` field. A note stores a term's path, such as
`Doing/Blocked`:

```wcl taxonomy:stage
label = "Stage"

term "Todo"

term "Doing" {
  colour = "blue"

  term "Blocked" {
    colour = "red"
  }
}

term "Done" {
  colour = "green"
}
```

A project, whose page lists its tasks through a query view with three views:

```wcl type:project
label  = "Project"
plural = "Projects"

field body {
  kind = "content"
}

template default {
  tab "Project" {
    slot "title"
    slot "body"

    query_view {
      title     = "Tasks"
      query     = "from task where project == this && !done sort due asc"
      allow_new = true

      view board {
        kind  = "kanban"
        group = "stage"
      }

      view dates {
        kind = "calendar"
        date = "due"
      }

      view rows {
        kind    = "table"
        columns = ["title", "stage", "due"]
      }
    }
  }
}
```

A task with one field of each common kind:

```wcl type:task
label  = "Task"
plural = "Tasks"

field stage {
  kind     = "taxonomy"
  taxonomy = "stage"
}

field priority {
  kind    = "select"
  options = ["low", "medium", "high"]
}

field estimate {
  kind = "int"
}

field cost {
  kind = "float"
}

field done {
  kind = "bool"
}

field due {
  kind = "date"
}

field remind_at {
  kind = "datetime"
}

field starts_at {
  kind = "time"
}

field span {
  kind = "date_range"
}

field link {
  kind = "url"
}

field project {
  kind     = "reference"
  target   = "project"
  relation = "two_way"
  inverse  = "tasks"
}

field owner {
  kind   = "reference"
  target = "person"
}

field tags {
  kind  = "list"
  items = "string"
}

field place {
  kind = "location"
  map  = "geo"
}

field cover {
  kind   = "asset"
  accept = "image"
}

field extra {
  kind   = "dict"
  values = "string"
}

field score {
  kind     = "int"
  computed = true
}

field body {
  kind = "content"
}

template default {
  tab "Task" {
    slot "title"
    slot "stage"
    slot "priority"
    slot "due"
    slot "project"
    slot "body"
  }
}
```

The rules these show, and that refusals most often name:

- A `taxonomy` field names a taxonomy that exists. A `select` field lists its `options`, and a note
  holds one of them exactly.
- A `reference` field names its `target` type, which must exist. `two_way` needs `inverse`: a
  field on the target type that names this field back. When the target has no field by that
  name, it gets one, so writing one side is enough: here `project` gains `tasks`. Set one side in
  notes too: the server writes the other.
- A list says what its items are with `items`, and a list of references also takes `target`.
- `computed` fields are for handlers, and cannot also be `required`.
- A view takes only its own items. `kanban` needs `group`, a single-term `taxonomy` field.
  `calendar` and `timeline` need `date`, a `date`, `datetime` or `date_range` field. `map` needs
  `location`. `table` takes `columns`. No view takes a filter: put the condition in the query's
  `where`.
- A template has at most one slot per field, and a slot names a field of the type or `title`.

## Every field kind in a note

```json note:task
{
  "fields": {
    "title": "Book the venue",
    "stage": "Doing/Blocked",
    "priority": "high",
    "estimate": 3,
    "cost": 420.5,
    "done": false,
    "due": "2026-12-01",
    "remind_at": "2026-11-30T09:00:00+11:00",
    "starts_at": "09:30",
    "span": { "start": "2026-11-20", "end": "2026-11-22" },
    "link": "https://example.com/venue",
    "project": "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC",
    "owner": "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB",
    "tags": ["venue", "budget"],
    "place": { "lat": -37.8136, "lng": 144.9631 },
    "cover": "01JB9A0C3D4E5F6G7H8J9K0M1N"
  },
  "blocks": [
    { "kind": "dict", "labels": ["extra"], "fields": { "supplier": "Town hall", "room": "Main" } },
    { "kind": "content", "labels": ["body"],
      "blocks": [ { "kind": "paragraph", "fields": { "text": "Ask for **two** quotes." } } ] }
  ]
}
```

- A date is `"YYYY-MM-DD"`. A datetime needs seconds and an offset: `Z` or `+11:00`. A time is
  `"HH:MM"` or `"HH:MM:SS"`.
- A date range is an object with both `start` and `end`, of the same form, `end` not before
  `start`.
- A reference, an asset and a mention hold a 26-character id that exists.
- A location is `{ "lat", "lng" }` for a `geo` map, with both members.
- `score` is computed, so the note leaves it out.

## Every content block

```json note:project
{
  "fields": { "title": "Office move" },
  "blocks": [
    { "kind": "content", "labels": ["body"],
      "blocks": [
        { "kind": "heading", "fields": { "level": 2, "text": "Plan" } },
        { "kind": "paragraph",
          "fields": { "text": "Ask [[01JB8Z4K2MQ7V3X9T5R6W8Y2ZB]] about *keys*, see [the lease](https://example.com/lease), and [[+task|Book the movers]]." } },
        { "kind": "list", "fields": { "style": "todo" },
          "blocks": [
            { "kind": "item", "fields": { "text": "Books", "checked": true } },
            { "kind": "item", "fields": { "text": "Desks" },
              "blocks": [ { "kind": "item", "fields": { "text": "Standing desk" } } ] }
          ] },
        { "kind": "list", "fields": { "style": "numbered" },
          "blocks": [ { "kind": "item", "fields": { "text": "Pack" } }, { "kind": "item", "fields": { "text": "Move" } } ] },
        { "kind": "quote", "fields": { "text": "Measure twice." } },
        { "kind": "divider" },
        { "kind": "table", "fields": { "header": true, "align": ["left", "right"] },
          "blocks": [
            { "kind": "row", "fields": { "cells": ["Item", "Cost"] } },
            { "kind": "row", "fields": { "cells": ["**Van**", "$400"] } }
          ] },
        { "kind": "toggle", "fields": { "level": 3, "text": "Details" },
          "blocks": [ { "kind": "paragraph", "fields": { "text": "Shown when opened." } } ] },
        { "kind": "admonition", "fields": { "kind": "warning", "title": "Notice is **30 days**" },
          "blocks": [ { "kind": "paragraph", "fields": { "text": "Give notice before the 1st." } } ] },
        { "kind": "image", "fields": { "asset": "01JB9A0C3D4E5F6G7H8J9K0M1N", "alt": "Floor plan", "caption": "Level 2" } },
        { "kind": "asset", "fields": { "asset": "01JB9A0C3D4E5F6G7H8J9K0M1N", "caption": "The plan as a file" } },
        { "kind": "card", "fields": { "note": "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB" } },
        { "kind": "code", "fields": { "language": "sh", "source": "just check" } }
      ] }
  ]
}
```

- `heading` needs `level`, 1, 2 or 3. `toggle` takes `level` 0 to 3.
- A list's `style` is `bullet`, `numbered` or `todo`. Its entries are `item` blocks, and an item
  nests items under `blocks`. `checked` is for `todo` lists.
- A table's rows are `row` blocks with `cells`, a list of inline markup. Every row has the same
  number of cells.
- A `divider` holds nothing.
- An `image` and an `asset` block name an asset id from `find_assets` or a download. A `card` names
  a note.
- In text: `**bold**`, `*italic*`, `[label](https://...)`, `[[<id>]]` or `[[<id>|label]]` to
  mention a note that exists, and `[[+<type>|<title>]]` for a note that does not exist yet.

## After a refusal

The error lists each issue with `at`, such as `fields.due` or `blocks[0].blocks[2]`, and what is
wrong. Fix exactly those, keep the rest, and propose again. For the value forms see
`value_format`, for blocks `block_format`, for type files `type_format`.
