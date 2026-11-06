---
name: vault-queries
description: Write queries in TheVault's query language for query_notes, query views and vault::query. Use before any query beyond a type and one simple condition, when a query failed with a syntax error, no such type or unknown field, and when you need a note's id and only know something about it.
---

# Writing vault queries

A query picks notes of one or more types and orders them. It is not SQL. The `query_format` tool
holds the whole grammar; this skill is the short version and worked examples. Every example here
is checked against the parser.

## The shape

```
from <type>[, <type>...] [include archived] [where <expr>] [search "<text>"]
  [sort <field> asc|desc, ...] [group by <field>] [select <field>, ...] [limit <n>]
```

Clauses come in that order, each at most once. Only `from` is required.

## Before you write one

1. Use the type's `name`, not its label: `list_types` gives the names (`task`, `email`, `person`).
2. Use the field names `get_type` lists for that type. Every note also has `id`, `type`, `title`,
   `created`, `updated` and `archived`.
3. Copy a select field's options and a taxonomy's term paths exactly (`get_taxonomy` lists terms).

## The rules that trip people up

- `==` compares, `=` is an error. `!=` is not equal.
- `&&` and `||` join conditions, `!` negates. `AND`, `OR` and `NOT` are errors.
- Text is in double quotes: `"Todo"`. Single quotes are errors.
- `sort due desc`, never `ORDER BY`. `contains(title, "plan")`, never `LIKE`.
- A missing field is false: `!done` finds notes with `done = false` and those without the field,
  while `done == false` misses the second kind.
- Lists use functions, not `==`: `has(tags, "x")`, `any(phones, |p| p.number == "+61...")`.
- A taxonomy field holds a path: `status == "Doing/Blocked"`, or `under(status, "Doing")` for the
  term and everything below it.
- Dates and datetimes compare with strings holding them, or with `today()`, `now()` and
  `days(n)`.
- A row holds the note's fields but never its body. Read the body with `get_note` on the row's
  `id`.
- `total` in the result is the count of everything that matched, even past the 50 rows returned.
  To count, query and read `total`.

## Worked examples

Open tasks due in the next week, soonest first:

```query
from task where !done && due <= today() + days(7) sort due asc
```

Tasks in a status or any status below it, as kanban columns:

```query
from task where under(status, "Active") group by status
```

A person by email address, whatever the case it was stored in:

```query
from person where any(emails, |e| has_ci(e.address, "ann@example.com"))
```

A person by phone number:

```query
from person where any(phones, |p| p.number == "+61400000000")
```

Unprocessed incoming mail, newest first, only the columns needed:

```query
from email where !processed && !under(labels, "system/SENT") sort sent desc select title, from_raw, sent
```

Incoming messages since the start of the month:

```query
from message where direction == "in" && sent >= "2026-11-01T00:00:00Z" sort sent desc
```

Unprocessed captures in the inbox:

```query
from fleeting where !processed sort created desc
```

Events that start on one day. `when` is a date range: `when.start` and `when.end` read its ends:

```query
from event where when.start >= "2026-11-06" && when.start < "2026-11-07" sort when.start asc
```

Notes of two types that mention a phrase, best match first:

```query
from email, message search "invoice overdue" limit 20
```

Every note of any type changed today:

```query
from * where updated >= today() sort updated desc limit 50
```

Projects with at least one open task (the tasks whose `project` points at the project):

```query
from project where any(linked_from(task, project), |t| !t.done) sort title asc
```

Tasks of a project, read through the reference:

```query
from task where project.title == "Website" && !done
```

Tasks with no due date, and archived ones too:

```query
from task include archived where empty(due) sort created desc
```

Only archived notes of a type:

```query
from task where archived sort updated desc
```

## Finding an id

Never make an id up or rebuild one from memory. Query for the note and copy the `id` from its row:

```query
from person where contains(title, "Rivera") select title
```

## After an error

- `unexpected character` or `expected ...`: the message gives the line and column. Fix that spot,
  usually one of the rules above.
- `there is no type "..."`: the name is wrong. Check `list_types`.
- `unknown field`: check the field names with `get_type`.
- `clauses come in the order ...`: move the clause to its place in the shape above.
