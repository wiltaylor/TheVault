---
id: '133'
title: vault backup fails with 'unable to open database file' when the archive's folder
  does not exist, so 'vault backup /data/backups/x.sqlar' fails on a fresh container
  until an upgrade has made /data/backups
blocked_by: []
status: done
verified: verified
---

# vault backup fails with 'unable to open database file' when the archive's folder does not exist, so 'vault backup /data/backups/x.sqlar' fails on a fresh container until an upgrade has made /data/backups

Seen in ticket 128: in the image, on a vault with no /data/backups yet, 'vault backup /data/backups/m.sqlar' prints {"error":"archive","message":"unable to open database file: /data/backups/m.partial"}. docs/install.md works around it with 'mkdir -p /data/backups && vault backup ...'. Either create the parent folder in vault backup, or have the entrypoint create /data/backups on every start, then simplify the command in docs/install.md.

## Worklog

### 2026-10-22T01:46:56Z — wil

Status set to todo.

### 2026-10-22T01:47:21Z — wil

Claimed.

### 2026-10-22T02:04:44Z — wil

Verified set to verified.

### 2026-10-22T02:04:44Z — wil

Status set to done.
