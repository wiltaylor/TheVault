---
id: '32'
title: 'writer: a crash after the commit point leaves state.db and the index behind
  the files'
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# writer: a crash after the commit point leaves state.db and the index behind the files

Found while building ticket 09. Writer::apply commits the files (step 7) and only then records versions in state.db (step 8) and updates the index (step 9). If the process dies after the commit manifest is written, Vault::open rolls the files forward, but state.db still reports the old version token for every changed note and history lacks the new content. The index is rebuilt on open, so only state.db is affected. Reproduce: crates/writer/tests/atomic.rs a_fault_after_the_commit_point_leaves_both_sides_changed_on_reopen (branch ticket/thevault-prd-09), then compare current_version with the file. One fix: on open, adopt a file whose text differs from its recorded head as a new version.

## Worklog

### 2026-10-12T14:40:20Z — wil

Triage (orchestrator): fix it. On Vault::open, after roll-forward, compare each note file with its recorded head in state.db and record any difference as a new version authored 'recovery'. Acceptance: the atomic.rs fault test then reports current_version matching the file and history holding the new content.

### 2026-10-12T14:40:20Z — wil

Status set to todo.

### 2026-10-12T14:40:49Z — /work

Claimed.

### 2026-10-12T14:55:53Z — wil

Verified set to verified.

### 2026-10-12T14:55:54Z — wil

Status set to done.

### 2026-10-12T14:55:55Z — wil

Fixed in 695d39d on ticket/thevault-prd-32. Writer::open calls recover() before the index rebuild: each notes/<type>/<ULID>.wcl whose text (or type folder) differs from State::head gets record_version authored Session("recovery") (writer::RECOVERED_BY); each head with no file is trash_note'd with that author. Files with no head are left to the existing lazy adoption (ADOPTED_BY), since a crashed create and a hand-copied file look the same. Added State::head and State::live_notes. Tests: atomic.rs fault-after-commit now checks current_version content == file, author recovery, history len 2, and that the recovered token works as a base; new tests for a delete interrupted after the commit point and for a clean reopen recording nothing. Cost: open now reads every note file once more and does one indexed state.db lookup per note, on top of the index rebuild's full read; worth watching in the S6 bench. Seen once under full-gate load: cli a_restored_backup_serves_the_same_notes_and_history hit testkit's 20s START_TIMEOUT; it passed alone in 13s and the rerun of the gate was green.

### 2026-10-12T15:13:39Z — wil

Merged into main as 00cc596 (fast-forward after merging main b594ad7 into the branch; just check green).
