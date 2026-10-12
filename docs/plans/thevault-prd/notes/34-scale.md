---
id: "34"
decision: D31
---

# Scale target

## Question
Q30: How large a vault?

## Answer
Design for 1M notes.

## Why
User chose option 3, larger than recommended.

## Consequences
Latency targets at this size are open (Q33). Index rebuild time, view pagination and directory sharding (D22) follow from it.
