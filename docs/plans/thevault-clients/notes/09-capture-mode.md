---
id: "09"
decision: D7
---

# Capture mode

## Question
Live, periodic catch-up, or both?

## Answer
Both: live receivers as events happen, a periodic catch-up from the providers since a stored cursor to cover anything missed while Android killed the app, and an optional first-run import of past calls and messages over a user-chosen date range, off by default.

## Why
Live alone misses events when the process is killed; catch-up alone is slow; a one-time backfill is a common need.

## Consequences
Every device event needs a stable provider id so live and catch-up paths de-duplicate.
