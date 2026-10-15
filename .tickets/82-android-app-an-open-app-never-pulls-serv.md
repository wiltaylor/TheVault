---
id: '82'
title: 'Android app: an open app never pulls server changes, so a person renamed in
  the web app reaches the phone contact only after the app is reopened'
plan: thevault-clients
blocked_by: []
status: done
verified: unmet
---

# Android app: an open app never pulls server changes, so a person renamed in the web app reaches the phone contact only after the app is reopened

Found by the emulator run of docs/android-manual-checks.md on main (b90c8f8, after ticket 78), check 37.

Repro: pair, finish capture setup, turn on Contacts write and enable upsert_contact and delete_contact for the phone. Have a person linked to a phone contact. Leave the app open in the foreground. Rename the person in the web app.

Expected (check 37): the phone contact's name changes to match within a minute.

Actual: nothing changes while the app stays open. The contact is renamed only once the app goes to the background and comes back (a sync round on resume), about 66 s in the run; with the app closed it would wait for the 15-minute SyncWorker. docs/sync.md lists the round triggers (open/foreground, 1.5 s after a local write, every 15 minutes); none fires for a change made on the server, and the server pushes no event for note changes to the phone (UnifiedPush carries notification and device.command events only).

Fix direction: have the server push a note.changed (or a generic sync) event to devices over UnifiedPush and run a round on it, and/or have the foreground app poll /api/sync/changes or subscribe to /api/events while open. Alternatively relax check 37 to 'after the next sync'.

Evidence: .scratch/emulator/RESULTS.md check 37.

## Worklog

### 2026-10-15T13:22:09Z — wil

Status set to claimed.

### 2026-10-15T13:22:09Z — wil

Fixed in 70490f8: the push dispatcher sends coalesced note.changed (1 s then at most every 10 s, skipping the originating device); the open Android app and the desktop app follow /api/events and pull 1 s after note.changed and on reconnect; a closed phone schedules SyncWorker.soon on the push. Shared parser in crates/sync/src/live.rs. check, android-test and desktop-test pass after rebase onto main. Emulator confirmation pending.

### 2026-10-15T13:22:09Z — wil

Verified set to unmet.

### 2026-10-15T13:22:09Z — wil

Status set to done.
