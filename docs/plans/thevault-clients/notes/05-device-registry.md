---
id: "05"
decision: D3
---

# Device registry

## Question
Does the server record each app install as a device?

## Answer
Yes. A `devices` table in `state.db` holds id, name, platform, capabilities, token id, push endpoint and last-seen time. Settings > Devices lists them and lets the user rename or revoke one; revoking deletes its token.

## Why
Push, device-targeted commands and revoking a lost phone all need a stable device identity; tokens alone carry no platform or push address.

## Consequences
Device events and notifications record which device they came from or went to.
