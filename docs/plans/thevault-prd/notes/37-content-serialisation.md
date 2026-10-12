---
id: "37"
decision: D34
---

# Content block serialisation

## Question
Q34: How is a content field's block tree written in WCL?

## Answer
As structured WCL blocks (`heading`, `paragraph`, `code`, `terminal`, `query_view { query = "..." view = "kanban" }`, and so on). Inline formatting (bold, italic, links, note mentions) uses a small inline markup inside paragraph strings.

## Why
Query, map and canvas blocks are structured data. Markdown directives would give agents a second syntax and could not be schema-validated.

## Consequences
The inline markup grammar is defined in the PRD's content section. Note mentions in paragraphs are ULID references, indexed like reference fields.
