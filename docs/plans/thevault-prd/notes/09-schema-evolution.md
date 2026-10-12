---
id: "09"
decision: D6
---

# Schema evolution

## Question
Q9: How do existing notes follow schema changes?

## Answer
Simple changes (add, rename, remove field) apply eagerly to all notes after a preview. Complex changes use a wscript migration. Unknown data is kept and flagged. Adding a required field requires a default value that existing notes receive. Adding an optional field leaves existing notes without a value.

## Why
Lazy migration creates a mixed-shape vault and silently wrong queries. The default-or-optional rule was added by the user.

## Consequences
Schema changes rewrite many files, so they must be atomic and versioned like any other bulk write.
