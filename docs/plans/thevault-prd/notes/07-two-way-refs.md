---
id: "07"
decision: D4
---

# Two-way and peer reference storage

## Question
Q7: Store reverse side or derive it?

## Answer
Both sides are stored in the notes' files. The server writes both sides in one atomic operation. Peer relations store the list on every member.

## Why
An agent reading one file sees all relationships, and git diffs show both ends.

## Consequences
A single logical edit touches several files, so atomic multi-file writes and crash recovery are required (accepted unknown: mechanism). Optimistic versioning (D12) must cover every file touched.
