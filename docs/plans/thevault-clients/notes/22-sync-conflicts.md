---
id: "22"
decision: D20
---

# Sync and conflicts

## Question
How do changes flow and how are conflicts handled?

## Answer
The home server is the master copy. A change feed `GET /api/sync/changes?since=<seq>` built on `versions.seq` gives devices what changed (notes, types, taxonomies, scripts, tasks, deletions). Devices replay queued edits through the normal write API with their base version. On a 409 the device three-way merges by field and by top-level block; a true clash keeps the server's version, saves the device's as a conflict copy titled "<title> (conflict, <device>)", and raises a notification. Types, templates, taxonomies, scripts and tasks are editable only online.

## Why
Nothing is lost silently; most offline edits merge cleanly; keeping definitions server-only avoids schema conflicts.

## Consequences
Blocks need a stable identity for block-level merge; today blocks have no ids (to verify in SPEC). Conflict copies need a link to the original.
