---
id: "17"
decision: D15
---

# Vault data model for calls and messages

## Question
How are calls and messages stored?

## Answer
Built-in types, customisable but not deletable, filled by default device scripts the user can edit: `message` (one note per message: channel sms|mms|rcs, direction, from and to as `person` references plus the raw number or name, thread, sent time, text, attachments as assets, `partial` for RCS), `call` (one note per call: direction, person and number, duration, missed), and `conversation` (one note per thread showing its messages in a query view).

## Why
One note per message makes cross-cutting questions plain queries; tens of thousands of notes is within the server's tested range. Default scripts give a working setup with user control.

## Consequences
Built-in type handling extends to three more types. Retention and exclusions become statable (Q28). Volume grows by thousands of notes a month; the deferred optimisation pass should include this load.
