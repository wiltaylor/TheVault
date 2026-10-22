---
id: '148'
title: Shipped-defaults seeding adds a new field's slot to the card template too
blocked_by: []
status: todo
verified: unknown
---

# Shipped-defaults seeding adds a new field's slot to the card template too

Ticket 147 added processed to the email and message types with a slot on both the default and card templates. The seeding in crates/writer/src/shipped.rs (with_shipped_fields) only adds slots to the vault's default template, so vaults from before 0.1.13 get processed on the page but not on the card, and cannot tick it from a card in query views or kanban until the user adds slot "processed" to template card. Decide whether the seeding should also place a new field's slot in a vault template of the same name as a shipped one that places it (card), which would also give the person card its photo slot on old starter vaults (test the_oldest_starter_vault_gains_the_person_fields_and_the_device_files asserts it does not today).

## Worklog

### 2026-10-22T02:09:27Z — wil

Status set to todo.
