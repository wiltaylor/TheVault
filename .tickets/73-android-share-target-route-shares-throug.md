---
id: '73'
title: 'Android share target: route shares through share_received and device_share.ws
  once the device runtime lands'
plan: thevault-clients
blocked_by:
- '61'
- '68'
status: parked
verified: unknown
park_cause: needs-triage
---

# Android share target: route shares through share_received and device_share.ws once the device runtime lands

Ticket 68 made the Android share target build the share_received payload ({text, subject, url, attachments}) in plugins/tauri-plugin-vault-device/src/share.rs and create the fleeting note directly, because the device runtime (ticket 61) was not merged. Once both are on main: enqueue share_received through 61's device event queue instead of calling share::fleeting_note, so the default scripts/device_share.ws (and the user's handler) decides the note. Make sure exactly one note is made per share, and keep the Robolectric S5 ShareTargetTest green (it asserts a queued fleeting note with assets).

## Worklog
