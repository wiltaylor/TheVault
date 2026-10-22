---
id: '110'
title: google_event.ws sends attendees as bare { email } objects when attendees_raw
  changes, so Google replaces the attendee list and drops each attendee's responseStatus,
  displayName and optional flag; a PATCH should keep Google's entries for addresses
  it keeps (needs a GET or the snapshot to hold them)
blocked_by: []
status: done
verified: verified
---

# google_event.ws sends attendees as bare { email } objects when attendees_raw changes, so Google replaces the attendee list and drops each attendee's responseStatus, displayName and optional flag; a PATCH should keep Google's entries for addresses it keeps (needs a GET or the snapshot to hold them)

## Worklog

### 2026-10-22T01:46:56Z — wil

Status set to todo.

### 2026-10-22T01:47:42Z — wil

Claimed.

### 2026-10-22T02:04:49Z — wil

Done on ticket/109 (1759518): google_event.ws GETs the event before a PATCH that changes attendees (or uses the 412 GET) and keeps Google's whole entry per kept address, ignoring case; test changing_the_attendees_keeps_googles_entry_for_each_address_it_keeps fails on the old script.

### 2026-10-22T02:04:50Z — wil

Verified set to verified.

### 2026-10-22T02:04:50Z — wil

Status set to done.
