---
id: "27"
decision: D24
---

# Git

## Question
Q23: Does the app use git?

## Answer
The app never touches git. The user may put the vault folder in git, with `.vault/` ignored.

## Why
D13 already provides history. Two history systems means two sources of truth.

## Consequences
None.
