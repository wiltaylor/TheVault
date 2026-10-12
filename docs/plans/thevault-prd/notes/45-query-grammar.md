---
id: "45"
decision: D42
---

# Query grammar

## Question
OQ4b: What is the exact query grammar?

## Answer
Clauses in fixed order: `from <type>[, <type>]` or `from *`, then optional `where <expr>`, `search "<text>"`, `sort <field> [asc|desc], ...`, `group by <field>`, `select <field>, ...`, `limit <n>`. Expressions use wscript syntax (`==`, `!=`, `<`, `&&`, `||`, `!`, closures). Host functions: `today()`, `now()`, `days(n)`, `has(list, value)`, `under(taxonomy_field, "term")`, `any(list, |x| expr)`, `all(list, |x| expr)`, `linked_from(type, field)`. A dot follows a single reference (`project.status`). `this` is the note whose template or content holds the query. Pagination is a cursor in the API, not part of the grammar.

## Why
Defaulted: the user asked the agent to design it. wscript is Rust-like, so the PRD's earlier example using `=` and `and` was wrong and is corrected.

## Consequences
`this` lets a type's template show related notes, for example a Project template listing its tasks.
