---
id: '81'
title: 'Contacts: a person created in the web app never becomes a phone contact, since
  nothing queues upsert_contact for an unlinked person'
plan: thevault-clients
blocked_by: []
status: done
verified: unmet
---

# Contacts: a person created in the web app never becomes a phone contact, since nothing queues upsert_contact for an unlinked person

Found by the emulator run of docs/android-manual-checks.md (check 38; Android 15 API 35 emulator, debug APK at 1574931 with the ticket 76 workaround).

Repro: pair the phone and finish capture setup; in Settings > Devices turn on Contacts write and enable upsert_contact and delete_contact; create a person note with a phone number in the web app (POST /api/notes?type=person {title: 'Carla Created', phones: ['+16502530005']}); let the phone sync (bring the app to the foreground).

Expected (check 38): a matching contact appears on the phone.

Actual: no contact appears, even after a sync round and 90 s. GET /api/devices/<id>/commands lists no upsert_contact command. Renaming an already-linked person does reach the phone (check 37, after a round), because a linked person's change is queued as a contact_changed event on the phone; an unlinked person has no contact, so no event, and the default scripts queue no upsert_contact for it: device_contacts.ws only implements the command, and no person handler on the server calls vault::device_command('upsert_contact', ...).

Fix direction: a default after_create/after_update handler for person (or a step in the phone's pull) that queues upsert_contact for a person with phones and no link to that device, when the device has contacts write on and the command enabled; or document that new people do not reach the phone and change check 38.

Evidence: .scratch/emulator/RESULTS.md check 38.

## Worklog

### 2026-10-15T13:14:01Z — wil

Status set to claimed.

### 2026-10-15T13:14:01Z — wil

Fixed in 5b2b34a: the writer queues upsert_contact for each eligible device (contacts write on, command enabled) when a person with a phone or address is created or its contact fields change and it is not linked on that device; no echo to the writing device; the server links device_contacts when the device reports done. Linked people keep flowing via contact sync, not a full overwrite (D16). check and android-test pass (android-test passed on rerun; first run had 2 OnboardingActivityTest failures 'cannot reach the server' at load avg ~16). Emulator confirmation pending.

### 2026-10-15T13:14:01Z — wil

Verified set to unmet.

### 2026-10-15T13:14:01Z — wil

Status set to done.
