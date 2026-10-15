---
id: '80'
title: 'Android MMS: a captured MMS image shows as a file chip in the message note,
  not as the image'
plan: thevault-clients
blocked_by: []
status: done
verified: unmet
---

# Android MMS: a captured MMS image shows as a file chip in the message note, not as the image

Found by the emulator run of docs/android-manual-checks.md (check 31: 'A message note in the thread's conversation with the image attached and shown').

Repro: receive an MMS with an image on the paired phone (in the emulator run, the MMS is inserted into the Telephony provider by the test stand-in for Google Messages), then open the message note in the app or the web app.

Expected: the image shows in the note.

Actual: the image is attached (the message's attachments field holds the asset photo.png, mime image/png), but the message type's attachments field (kind asset, multiple) renders as a 'photo.png' chip with Upload, so no image is shown. Fleeting notes from shares, by contrast, have an image field and show their images.

Fix direction: either render image assets in asset fields as thumbnails (web client), or have device_messages.ws also put image attachments into an image block/field of the message.

Evidence: .scratch/emulator/run-workaround/shots/31-mms-note.png.

## Worklog

### 2026-10-15T13:05:55Z — wil

Status set to claimed.

### 2026-10-15T13:05:55Z — wil

Fixed in 7e76612: message templates show images from attachments (image { field = "attachments" }) and keep the slot for editing; MMS parts and shared files get an extension from their content type. check and android-test pass. Known side effects: every message note without attachments shows an empty add-image box; existing vaults keep the old message.wcl; HEIC is not in the web image list. Emulator confirmation pending.

### 2026-10-15T13:05:55Z — wil

Verified set to unmet.

### 2026-10-15T13:05:55Z — wil

Status set to done.
