---
id: "27"
decision: D24
---

# Matching people

## Question
How do phone numbers and names map to `person` notes?

## Answer
`person` gains `phones` and `addresses` lists. Numbers are stored in E.164, normalised with the phone's region; an incoming number matches the `person` holding it. An unknown number creates no person: the message keeps the raw number and the conversation offers Add contact. RCS senders, which arrive as display names, match by exact name or the phone contact's display name. Each `person` holds a hidden per-device contact id; the phone contact stores the note id in a custom data row.

## Why
E.164 matching is the standard approach; not auto-creating people keeps spam out; RCS gives names only (D13).

## Consequences
Number normalisation needs a phone-number library on the device (libphonenumber). Name matching for RCS can mis-match people with the same name; acceptable given D13 marks RCS partial.
