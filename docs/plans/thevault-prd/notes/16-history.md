---
id: "16"
decision: D13
---

# Version history

## Question
Q16: How is history kept?

## Answer
An internal version store inside the app, not git. The UI shows per-note history and restores versions. Deleted notes go to a trash.

## Why
User chose option 2 over git commits.

## Consequences
The index is no longer the only state outside the WCL files, so where history lives and how it is retained must be decided (Q24). Whether the app touches git at all is open (Q23).
