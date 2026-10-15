---
id: "18"
decision: D16
---

# Contacts sync

## Question
Which direction do contacts sync?

## Answer
Two-way between phone contacts and `person` notes. Each side stores the other's id. Conflicts resolve per field, most recent edit wins. Deleting on one side unlinks the other; deletion syncing is an opt-in.

## Why
The user chose two-way. One address book; the default cannot lose a person.

## Consequences
`person` gains phone numbers, addresses and a device contact id per device (Q14). The phone side needs a place to store the vault note id (Q14).
