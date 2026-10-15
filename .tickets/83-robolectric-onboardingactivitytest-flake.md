---
id: '83'
title: 'Robolectric OnboardingActivityTest flakes under host load: pairing tests fail
  with ''cannot reach the server'' (load avg ~16), pass on rerun'
plan: thevault-clients
blocked_by: []
status: done
verified: verified
---

# Robolectric OnboardingActivityTest flakes under host load: pairing tests fail with 'cannot reach the server' (load avg ~16), pass on rerun

## Worklog

### 2026-10-15T16:56:28Z — wil

Status set to claimed.

### 2026-10-15T16:56:28Z — wil

Vault opening inside the 10 s pairing wait dominated under disk load; the test now opens the vault before pairing and unlinks after each test. 10 in a row pass under load.

### 2026-10-15T16:56:29Z — wil

Verified set to verified.

### 2026-10-15T16:56:29Z — wil

Status set to done.
