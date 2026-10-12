---
id: "21"
decision: D18
---

# Clients

## Question
Q21: Which clients are supported?

## Answer
Responsive web app installable as a PWA, with stylus input through pointer events. No offline mode. Native mobile and desktop clients come later and are out of scope.

## Why
Ink is mostly used on tablets. Offline needs sync and merge.

## Consequences
The API must be client-agnostic so later native clients need no server changes.
