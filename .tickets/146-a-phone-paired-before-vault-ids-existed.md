---
id: '146'
title: 'A phone paired before vault ids existed never starts fresh: its first 0.1.13-alpha.3
  sync records the current server''s vault id even when its local vault and queue
  came from another vault'
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# A phone paired before vault ids existed never starts fresh: its first 0.1.13-alpha.3 sync records the current server's vault id even when its local vault and queue came from another vault

Seen 2026-10-21 on the user's phone (SM-F976B) paired with vault.wil.dev at 06:27 under 0.1.12, with a local vault and a 784-change queue that came from a previous dev vault (via phone contact links). After installing 0.1.13-alpha.3 (ticket 143) no fresh start happened: an existing pairing carries no vault id, so the first sync recorded severus's id, and the old queue was pushed as creates under the old vault's ids (158 person notes 01M54...). The user chose to keep the data. Possible fix: on the first sync that records a vault id for a local vault with synced notes or a queue, compare the queued notes' bases (or a recorded server URL, or the cursor) against the server and treat a vault whose bases the server never had as foreign; or record the vault id at pairing for every future pairing (done in 143) and accept this one-off for pre-143 pairings.

## Worklog
