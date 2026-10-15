---
id: "10"
decision: D8
---

# Offline

## Question
What works when the server is unreachable?

## Answer
Full offline editing with sync, chosen by the user over the recommended queue-only option.

## Why
The user wants the apps usable without a connection.

## Consequences
This is the largest decision in the map. It reopens front-end delivery (D1, Q21) and needs: the offline data scope (Q22), a sync protocol with change feed and conflict handling (Q23), whether desktop is offline too (Q24). The original PRD made offline a non-goal (D18 there) because it needs sync and merge; that cost now applies. Device scripts must run against the local replica when offline.
