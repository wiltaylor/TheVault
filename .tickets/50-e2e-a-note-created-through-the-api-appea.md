---
id: '50'
title: 'e2e: ''a note created through the API appears in its type list'' can miss
  its row while the 10,000-note bulk test runs'
blocked_by: []
status: done
verified: verified
---

# e2e: 'a note created through the API appears in its type list' can miss its row while the 10,000-note bulk test runs

Seen once in 4 full runs of ./dev just test-e2e on feature/theme-settings-types (shell.spec.ts:48). The test opens the Tasks list sorted by title and waits for the row 'Tabbed …'. While the parallel bulk test has added its 10,000 notes, that row can sit outside the virtualised window, so it is never rendered. Fix by searching for the note or scrolling the list to it, rather than relying on it being near the top.

## Worklog

### 2026-10-22T02:09:27Z — wil

Status set to todo.

### 2026-10-22T02:10:18Z — wil

Claimed.

### 2026-10-22T02:22:04Z — wil

Cause now: the list is no longer virtualised; a type over MAX_LISTED (200) tasks lists none, and the shared server keeps adding tasks. Test now searches the Tasks list for its unique title and expects '1 match'. With 70 filler tasks per worker the old test failed 5/5 and the new passed 5/5; plain --repeat-each=5 with the bulk test 10/10. Commit 5dff002 on ticket/50.

### 2026-10-22T02:22:04Z — wil

Verified set to verified.

### 2026-10-22T02:22:04Z — wil

Status set to done.
