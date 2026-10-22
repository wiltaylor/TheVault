---
id: '120'
title: A field rename in a type change leaves template query views naming the old
  field, and template put accepts query_view columns and where-clause fields no queried
  type has
blocked_by: []
status: todo
verified: unknown
---

# A field rename in a type change leaves template query views naming the old field, and template put accepts query_view columns and where-clause fields no queried type has

Found during ticket 118. On a type invoice with field amount and a default template whose query_view has query "from invoice where !paid" and a table view with columns ["title", "amount", "received"], `vault type change invoice -f renamed.wcl --rename amount=total` applied cleanly, renamed slots and note values, but left columns = ["amount"] in the template. Separately, `vault template put invoice card` accepted a query_view with query "from invoice where nonexistent == 1" and columns ["title", "nonexistent"] with no warning, while slot "nope" is refused. docs/vault-format.md says a rename renames controls bound to the field; it says nothing of query views. Decide whether renames should rewrite query views of the type's own templates and whether template validation should check fields of a single named type.

## Worklog

### 2026-10-22T02:09:27Z — wil

Status set to todo.
