---
id: '84'
title: 'Android app: device settings changed in Settings > Devices (contacts write,
  commands) do not reach an open app, so contact edits from the web app are dropped
  until it is reopened'
plan: thevault-clients
blocked_by: []
status: done
verified: verified
---

# Android app: device settings changed in Settings > Devices (contacts write, commands) do not reach an open app, so contact edits from the web app are dropped until it is reopened

Found by the emulator run of docs/android-manual-checks.md on main (8404c46, after tickets 78 and 82), check 37.

Repro (emulator, app open in the foreground the whole time): a person linked to a phone contact; in Settings > Devices turn on Contacts write and enable upsert_contact and delete_contact (PATCH /api/devices/<id>); rename the person in the web app.

Expected (check 37): the phone contact is renamed within a minute.

Actual: the app pulls the renamed person within about a second (ticket 82 works: a note created on the server was in the open app's vault 1.05 s later), but the contact is not renamed, not even after a second rename a minute later. The phone's device runtime still holds the old settings: state.db device_runtime 'settings' = {"commands":[],"contacts_write":false,...} after the PATCH. Once the app goes to the background and comes back, the settings refresh, the queued contact change applies (contact became 'Zed Two'), and from then on a rename made with the app open reaches the contact in 2.8 s.

So the open app refreshes its device settings only when it comes to the front (openAndSync), not on live rounds; a switch turned on while the app is open has no effect until it is reopened, and the contact changes processed meanwhile are recorded as skipped rather than retried. The background path (SyncWorker / note.changed push) may have the same gap.

Fix direction: refresh the device's settings from the server in every sync round (or on a device.updated event over /api/events and UnifiedPush), before running contact propagation.

Evidence: .scratch/emulator/RESULTS.md check 37; .scratch/emulator/run-final/logs/37-capture-state.txt (contact_changed events applied only at 13:37:57, the resume).

## Worklog

### 2026-10-15T15:17:04Z — wil

Status set to claimed.

### 2026-10-15T15:17:05Z — wil

Fixed in 8a7609d: PATCH of device settings publishes device.settings on /api/events and pushes it (coalesced) to that device only; every sync round fetches settings first; open apps run a round on the event; background SyncWorker opens the device runtime so contacts catch up. Emulator check 37 passes (contact renamed 5.2 s after the change); check and android-test pass after rebase.

### 2026-10-15T15:17:05Z — wil

Verified set to verified.

### 2026-10-15T15:17:06Z — wil

Status set to done.
