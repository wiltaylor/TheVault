---
id: "12"
decision: D9
---

# Event handler timing

## Question
Q12: Pre-commit or post-commit handlers?

## Answer
`before_create/update/delete` run synchronously in the save and can modify or reject it. `after_*` run from a queue after commit for side effects. Cascades are bounded by a depth limit with cycle detection.

## Why
Computed fields need before handlers. wscript has no async, so slow work belongs in queued after handlers.

## Consequences
The host API available to handlers is undecided (Q29). Depth limit value is an accepted unknown.
