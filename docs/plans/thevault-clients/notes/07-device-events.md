---
id: "07"
decision: D5
---

# Device event model

## Question
What device events exist, where are handlers declared, and where do they run?

## Answer
A fixed catalogue: `sms_received`, `sms_sent`, `mms_received`, `rcs_received` (D13), `call_ended`, `contact_changed`, `contact_deleted`, `share_received`. Device events run on the device; note events and scheduled tasks stay on the server. Handlers are declared in a vault file per platform, `devices/android.wcl`, naming the script and function per event. Scripts live in `scripts/*.ws` in the vault and are synced to the device.

## Why
Matches the user's rule (server by default, device for device-specific events) without a per-handler location setting to get wrong. One source of truth for scripts.

## Consequences
The schema and docs need the new file kind and event names. Device-run scripts need their own module set (Q27).
