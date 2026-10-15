---
id: "25"
decision: D23
---

# The device module

## Question
Which device functions exist?

## Answer
Reading: `sms.list`/`get`, `calls.list`, `contacts.list`/`get`, available when Android granted the permission. Writing: `contacts.upsert`/`delete`. Sending: `sms.send` (needs SEND_SMS). Writing and sending each need a per-device switch in Settings > Devices.

## Why
Reading copies data in; writing changes the phone; sending reaches other people.

## Consequences
SEND_SMS is a further restricted permission for sideloaded apps; onboarding covers it when the switch is turned on.
