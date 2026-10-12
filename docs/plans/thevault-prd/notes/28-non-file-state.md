---
id: "28"
decision: D25
---

# Non-file state

## Question
Q24: Where does state outside the WCL files live?

## Answer
`.vault/index.db` is rebuildable from the files. `.vault/state.db` is not, and holds version history, encrypted secrets, API tokens and task run logs. History is kept forever, with optional pruning by age or count. `vault backup` archives files and `state.db` together.

## Why
Separating them makes it obvious what must be backed up and what can be deleted.

## Consequences
Restoring files without `state.db` loses history, secrets and tokens. The PRD states this.
