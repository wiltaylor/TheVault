---
id: '149'
title: 'The image''s pre-upgrade backup is skipped when moving from a pre-release
  to its stable release: entrypoint.sh compares versions with sort -V, which ranks
  0.1.13-alpha.4 above 0.1.13'
blocked_by: []
status: todo
verified: unknown
---

# The image's pre-upgrade backup is skipped when moving from a pre-release to its stable release: entrypoint.sh compares versions with sort -V, which ranks 0.1.13-alpha.4 above 0.1.13

Seen 2026-10-22 upgrading severus from 0.1.13-alpha.4 to 0.1.13: no backup was written and /data/.vault/server-version stayed 0.1.13-alpha.4, because docker/entrypoint.sh line ~69 picks the newest of the recorded and image versions with 'sort -V | tail -n 1', and sort -V sorts 0.1.13 before 0.1.13-alpha.4, so the upgrade looked like a downgrade. Fix: compare with semver rules (a pre-release sorts before its release), for example a small comparison in the vault CLI (the semver compare in crates/sync/src/gate.rs exists) invoked by the entrypoint, or shell that splits the pre-release; record the image version after the comparison either way; add the alpha-to-stable and stable-to-next-alpha cases to docker/test-image.sh.

## Worklog

### 2026-10-22T01:46:56Z — wil

Status set to todo.
