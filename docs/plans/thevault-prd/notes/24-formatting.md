---
id: "24"
decision: D21
---

# Canonical formatting

## Question
Q4: Canonical format or format-preserving edits?

## Answer
The vault owns a canonical format. Every write from the UI or submitted by an agent is reformatted. Comments in vault files are not preserved.

## Why
Agents submit WCL through the API, so files only ever take the canonical shape and diffs stay stable. Upstream format-preserving editing would block this project.

## Consequences
Agents must re-read a note after writing to see the canonical form. The API returns it in the write response.
