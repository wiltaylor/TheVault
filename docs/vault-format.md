# Vault file format

This document defines the WCL vocabulary of every file in a vault: notes, types with their templates and handlers, taxonomies, scheduled tasks, and the content, ink and canvas blocks that notes and templates contain. It settles OQ-S1 and overrides the illustrative note example in `docs/plans/thevault-prd/SPEC.md`.

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
- [Validating vault files](#validating-vault-files)

## Files at a glance

| File kind | Path in the vault | Structural schema | Example |
|-----------|-------------------|-------------------|---------|
| Note | `notes/<type>/<ULID>.wcl` | `schema/common.wcl` plus a generated note schema | `vault/notes/specimen/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA.wcl` |
| Type | `types/<type>.wcl` | `schema/common.wcl` plus `schema/type.wcl` | `vault/types/specimen.wcl` |
| Taxonomy | `taxonomies/<taxonomy>.wcl` | `schema/taxonomy.wcl` | `vault/taxonomies/status.wcl` |
| Task | `tasks/<task>.wcl` | `schema/task.wcl` | `vault/tasks/daily_digest.wcl` |
| Canvas | A `canvas` content block, or a `canvas` field value in a note | `schema/common.wcl` | `vault/notes/board/01JB8Z4K2MQ7V3X9T5R6W8Y2ZF.wcl` |

Paths in the last two columns are relative to `docs/vault-format/`. Scripts (`scripts/*.ws`) are wscript, not WCL, and this document does not define them. Assets (`assets/<ULID>/<filename>`) are stored byte for byte.

The example vault under `docs/vault-format/vault/` is self-consistent: its notes use its types, and its types use its taxonomies. Notes that the examples reference but do not need (the parent, child and peer specimens) are left out. The scripts named by the example handlers and task are left out too.

## Names and identifiers

| Name | Rule | Where it comes from |
|------|------|---------------------|
| Type name | `[a-z][a-z0-9_]*`, at most 64 characters | The file stem of `types/<type>.wcl`, and the folder name under `notes/` |
| Taxonomy name | Same rule as a type name | The file stem of `taxonomies/<taxonomy>.wcl` |
| Task name | Same rule as a type name | The file stem of `tasks/<task>.wcl` |
| Script name | Same rule as a type name | The file stem of `scripts/<script>.ws` |
| Field name | Same rule as a type name. `title`, `created` and `updated` are reserved | The label of a `field` block |
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
| Date range | record | `{ start: <date>, end: <date> }`, or two datetimes. Both ends inclusive, `end` not before `start` | `{ start: "2026-10-20", end: "2026-11-01" }` |
| Duration | string | ISO 8601 duration | `"PT1M"` |
| URL | string | An absolute URL with a scheme | `"https://example.com/importer"` |
| Note reference | string | The target note's ULID: 26 Crockford base32 characters, upper case | `"01JB8Z4K2MQ7V3X9T5R6W8Y2ZB"` |
| Asset reference | string | The asset's ULID, naming the folder `assets/<ULID>/` | `"01JB9A0C3D4E5F6G7H8J9K0M1N"` |
| Taxonomy term | string | The term's path from its root, names joined by `/` | `"Active/In progress"` |
| Geo location | record | `{ lat: <float>, lng: <float> }` in WGS 84 decimal degrees. `lat` in -90..90, `lng` in -180..180 | `{ lat: -37.8136, lng: 144.9631 }` |
| Image location | record | `{ x: <float>, y: <float> }` in pixels from the map image's top-left corner | `{ x: 412.0, y: 188.5 }` |
| Palette colour | string | One of `gray`, `brown`, `orange`, `yellow`, `green`, `blue`, `purple`, `pink`, `red` (OQ-S4) | `"blue"` |
| Ink colour | string | `#rrggbb`, lower-case hex | `"#1f2937"` |

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
| `of` | `"date"` or `"datetime"` | `"date"` |

```wcl
field span {
  kind = "date_range"
  of   = "date"
}
```

```wcl
span = { start: "2026-10-20", end: "2026-11-01" }
```

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
| `tab "<title>" { ... }` | block | At least one tab. A template with one tab shows no tab bar. Card views render only the first tab |

A tab holds an ordered mix of slots and content blocks:

| Item | Form | Meaning |
|------|------|---------|
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

### Field references

The text of a fixed content block in a template can show a field's value inline: `{{name}}` in any `text` item (a heading, paragraph, quote or list item, including one inside a canvas card), or in an image's `alt` or `caption`, renders the current value of field `name` of the note being shown, read-only. `name` is `title` or a field of the type; the schema rejects any other name with "`{{name}}` names no field of the type". A reference is not a slot: it places nothing, so the field still has no entry on the page and stays editable in the properties sidebar.

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
- An asset in the field that is not an image (by its file name: `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.avif` or `.svg`) is skipped. An empty field shows nothing on a card; on the note page it offers to add an image, which uploads the file and sets the field (for a multiple field, adds it).
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
| `allow_domains` | list of strings | `[]` | Host names that HTTP host functions may reach. `after_*` handlers only |
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

   Clients cannot set `created` or `updated`. The server ignores either value on a write and writes its own.
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

An attached file, shown as a download card with the file name from `assets/<ULID>/<filename>`.

| Item | Form | Meaning |
|------|------|---------|
| `asset` | asset reference | Required |
| `caption` | string | Caption shown with the file |

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

These text items use the inline markup in PRD Appendix A (D41): `heading.text`, `paragraph.text`, `item.text` and `quote.text`, wherever the block sits, a canvas card included. No other string is parsed as markup.

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
| `query` | string | Required. A query in the grammar of PRD Appendix B. `this` is the note that holds the block |
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

## Ink

An ink drawing is an `ink` block (D18, D29). It is a content block with no label, or a note's `ink` field with the field name as its label.

| Item | Form | Meaning |
|------|------|---------|
| `width`, `height` | int | Required. The drawing surface in ink units, which are CSS pixels at 100% zoom |
| `background` | string | `blank`, `lines`, `grid` or `dots`. Default `blank` |
| `stroke { ... }` | block | The strokes, in drawing order. Later strokes draw on top |

Each `stroke` holds:

| Item | Form | Meaning |
|------|------|---------|
| `tool` | string | Required. `pen`, or `highlighter`, which draws translucent and behind pen strokes |
| `colour` | ink colour | Required. `#rrggbb` |
| `size` | float | Required. The perfect-freehand base size in ink units |
| `points` | list of floats | Required. Flat triples `x, y, pressure`. `x` and `y` are ink units from the top-left corner. `pressure` is 0.0 to 1.0, and 0.5 where the device reports none. At least one triple |

The renderer feeds each stroke's points to perfect-freehand. Erasing deletes whole strokes, so no eraser strokes are stored. The writer rounds coordinates to two decimal places and pressure to three.

```wcl
ink {
  width      = 800
  height     = 300
  background = "lines"

  stroke {
    tool   = "pen"
    colour = "#1f2937"
    size   = 4.0
    points = [10.0, 20.0, 0.5, 14.5, 22.0, 0.6, 20.0, 25.5, 0.7]
  }
}
```

## Canvas

A canvas is TheVault's own WCL format, not JSON Canvas (D15, D26). It is a `canvas` block: a content block with no label, or a note's `canvas` field with the field name as its label.

| Item | Form | Meaning |
|------|------|---------|
| `height` | integer | The height of the canvas on the page, in screen pixels, from 160 to 4000. Default 480. Its width always fills the page or pane it is in |
| `viewport { ... }` | block | Required. The view saved with the canvas |
| `group <id> { ... }` | block | Labelled frames that hold nodes |
| `node <id> { ... }` | block | Items on the canvas, in z-order. Later nodes draw on top |
| `edge <id> { ... }` | block | Connections between nodes |

`<id>` is an identifier, unique across the groups, nodes and edges of one canvas. Coordinates are canvas units, floats, with `y` increasing downwards. A canvas in a template's fixed content shows at the template's `height` on every note.

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
| `allow_domains` | list of strings | `[]` | Host names that HTTP host functions may reach |
| `secrets` | list of strings | `[]` | Secret names the task may read |
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

## Validating vault files

A vault file is checked in three layers. Each catches what the one before cannot.

1. **Data-only.** `store` parses the file and rejects anything outside [the data-only subset](#the-data-only-subset). `docs/vault-format/data-only.mjs` is a reference implementation of this check.
2. **Structure.** `wcl_lang` validates the file against the WCL schema for its file kind. This catches unknown fields and blocks, wrong value types, missing block items, out-of-range heading levels, and canvas edges that name no node.
3. **Semantics.** `schema` checks what WCL cannot express. That covers required top-level fields, the encodings in [Value encodings](#value-encodings), and closed vocabularies such as kinds, styles and colours. It also checks that references and terms exist, and the cross-file rules: inverse fields, template slots and handler events.

### The schema files

`docs/vault-format/schema/` holds the structural schemas. Vault files carry no import, so a host puts the schema in front of the file: it registers the schema types in a `wcl_lang::Environment`, or it concatenates the schema source and the file.

| File | Declares |
|------|----------|
| `common.wcl` | Date range and location records, every content block, ink, query view, canvas, `columns`, `control`, `slot`, `content` and `dict`. No `@document` |
| `type.wcl` | The `@document` of a type file, with `field`, `template`, `tab` and `handler` |
| `taxonomy.wcl` | The `@document` of a taxonomy file |
| `task.wcl` | The `@document` of a task file |
| `notes/<type>.wcl` | A generated `@document` for the notes of one type |

### Generated note schemas

A note's top-level fields depend on its type, so its `@document` is generated from `types/<type>.wcl` by these rules. `schema/notes/specimen.wcl` and `schema/notes/board.wcl` are generated from the example types.

- The document type is named after the type in PascalCase with a `Note` suffix, for example `SpecimenNote`.
- `title`, `created` and `updated` are `utf8`.
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
