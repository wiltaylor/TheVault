---
id: '73'
title: 'Android share target: route shares through share_received and device_share.ws
  once the device runtime lands'
plan: thevault-clients
blocked_by:
- '61'
- '68'
status: done
verified: verified
---

# Android share target: route shares through share_received and device_share.ws once the device runtime lands

Ticket 68 made the Android share target build the share_received payload ({text, subject, url, attachments}) in plugins/tauri-plugin-vault-device/src/share.rs and create the fleeting note directly, because the device runtime (ticket 61) was not merged. Once both are on main: enqueue share_received through 61's device event queue instead of calling share::fleeting_note, so the default scripts/device_share.ws (and the user's handler) decides the note. Make sure exactly one note is made per share, and keep the Robolectric S5 ShareTargetTest green (it asserts a queued fleeting note with assets).

## Worklog

### 2026-10-15T09:56:40Z — wil

Status set to claimed.

### 2026-10-15T09:56:40Z — wil

Shares now queue share_received and run it at once through the device runtime, so scripts/device_share.ws or the user's handler decides the note. Fallback: with no runtime open or no share_received handler declared, the core writes the same fleeting note directly so a share is never lost; a failing handler leaves the event queued to retry and writes no fallback. Reproduced first (a custom device_share.ws was ignored), now passes. Behaviour change: only images go in the fleeting image field; other files become asset blocks. Verified: ./dev just check and ./dev just android-test pass on main.

### 2026-10-15T09:56:40Z — wil

Verified set to verified.

### 2026-10-15T09:56:40Z — wil

Status set to done.
