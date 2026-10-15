---
id: "11"
decision: D9
---

# Notification mirroring

## Question
What does dismissing an Android notification do?

## Answer
Swiping marks it read on the server and leaves it in the bell list; tapping opens the note and marks it read. Reading, dismissing or clearing it anywhere else cancels it on the phone through push (D4). Android notification ids derive from server notification ids.

## Why
Swipes are casual; deleting would lose the record.

## Consequences
Requires the server to push read/dismiss events per device.
