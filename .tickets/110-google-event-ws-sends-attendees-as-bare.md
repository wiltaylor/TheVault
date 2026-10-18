---
id: '110'
title: google_event.ws sends attendees as bare { email } objects when attendees_raw
  changes, so Google replaces the attendee list and drops each attendee's responseStatus,
  displayName and optional flag; a PATCH should keep Google's entries for addresses
  it keeps (needs a GET or the snapshot to hold them)
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# google_event.ws sends attendees as bare { email } objects when attendees_raw changes, so Google replaces the attendee list and drops each attendee's responseStatus, displayName and optional flag; a PATCH should keep Google's entries for addresses it keeps (needs a GET or the snapshot to hold them)

## Worklog
