---
id: '159'
title: 'Empty the note trash: only a signed-in user can purge, never a token, script
  or the AI'
plan: ai
blocked_by: []
status: done
verified: verified
---

# Empty the note trash: only a signed-in user can purge, never a token, script or the AI

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC-2.md section D21.

Add POST /api/trash/empty and DELETE /api/trash/{id} that purge for a password-login session only (403 human_only for every API token scope), the Trash page's Empty trash and Delete forever actions with confirm dialogs, and the docs and skill lines.

Acceptance criteria:
- crates/server tests: a session empties the trash and the notes and their history are gone; a session purges one note and the others stay; read, write and admin tokens each get 403 human_only on both routes and nothing is purged; restoring a purged note is a 404.
- An asset referenced only by a purged note moves to the asset trash.
- Web component tests: Empty trash confirms with the count and calls the route; Delete forever confirms and calls the route.
- The embedded server never purges on behalf of a device token (test whichever way it is handled).
- ./dev just check passes.

## Worklog

### 2026-10-22T21:14:15Z — wil

Verified set to verified.

### 2026-10-22T21:14:15Z — wil

Status set to done.
