---
id: "25"
decision: D22
---

# Vault file layout

## Question
Q5: Where do notes, definitions and assets live?

## Answer
`notes/<type>/<ULID>.wcl` with content blocks inline. `types/<type>.wcl` holds schema and templates. `taxonomies/*.wcl`, `scripts/*.ws`, `tasks/*.wcl`. Assets at `assets/<ULID>/<filename>`. App state in `.vault/`.

## Why
Types are the organising structure. User-chosen folders would compete with types and taxonomies.

## Consequences
At 1M notes (D31) a single type directory may hold hundreds of thousands of files. Subdirectory sharding is an accepted unknown.
