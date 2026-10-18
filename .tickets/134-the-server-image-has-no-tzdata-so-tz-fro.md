---
id: '134'
title: The server image has no tzdata, so TZ from deploy/.env has no effect on the
  container's clock
blocked_by: []
status: dropped
verified: unknown
---

# The server image has no tzdata, so TZ from deploy/.env has no effect on the container's clock

Seen in ticket 128: compose passes TZ as the ticket asked, but debian:bookworm-slim has no /usr/share/zoneinfo, so glibc falls back to UTC. The server itself uses chrono-tz and per-task timezone fields, so only logs and tools run in the container are affected. Decide whether to install tzdata in docker/Dockerfile or drop TZ from compose.yaml and .env.example.

## Worklog

### 2026-10-18T09:35:57Z — wil

Filed in error by ticket 128: the image does have /usr/share/zoneinfo (debian tzdata), and TZ=Australia/Melbourne makes date print AEDT in the container.

### 2026-10-18T09:35:57Z — /work

Dropped.
