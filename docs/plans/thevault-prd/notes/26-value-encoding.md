---
id: "26"
decision: D23
---

# Encoding dates, URLs and references

## Question
Q32: Native WCL types or validated strings?

## Answer
ISO 8601 strings for dates and datetimes, plain strings for URLs, ULID strings for references. TheVault's schema layer validates them.

## Why
No upstream WCL dependency, and WCL cross-file refs need imports, which D20 bans.

## Consequences
Location and date range are structured values (blocks or maps) validated the same way.
