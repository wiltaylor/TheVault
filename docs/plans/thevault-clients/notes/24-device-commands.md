---
id: "24"
decision: D22
---

# Server-to-device commands

## Question
Can a server script ask a device to act?

## Answer
Yes, through an allowlist. A server script calls `vault::device_command(device, name, payload)`; the command is stored on the server, delivered by push (D4), and the device replies with a result. A device runs only commands its `devices/android.wcl` declares and the user enabled for that device in Settings > Devices.

## Why
Enables server-driven actions (send a text, update contacts) while bounding the damage of a buggy script.

## Consequences
Commands need ids, expiry and result storage on the server.
