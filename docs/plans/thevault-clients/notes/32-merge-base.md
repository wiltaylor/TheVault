---
id: "32"
decision: D29
---

# Merge base preservation

## Question
How does a device keep its merge base?

## Answer
The device keeps its own copy of the base text for every note edited offline. On the server, writes from a device always create a new version (never merged into the head), and pruning never deletes a version a registered device still depends on.

## Why
Browser-session merging and pruning can otherwise remove the base (note 26).

## Consequences
The server tracks, per device, the oldest base version still pending.
