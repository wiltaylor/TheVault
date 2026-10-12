---
id: '38'
title: 'vault-server start rebuilds the whole index: 18 min at 1M notes'
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# vault-server start rebuilds the whole index: 18 min at 1M notes

Found by ticket 30's S6 bench. Writer::open always calls index.rebuild, so vault-server took 1094.5 s to become ready on a 1M-note vault, after vault reindex had already built the index (899.5 s). No PRD target covers server start, but every restart costs about 18 minutes at the D31 scale. Needs a decision on whether startup can trust an existing index.

## Worklog

### 2026-10-12T15:56:13Z — wil

Triage (orchestrator): Decision: server start must not rebuild a current index. Record an index generation (the last applied commit id from the store journal) in index.db; on open, rebuild only when it is missing or behind, and catch up incrementally when possible. vault reindex still forces a full rebuild. Acceptance: vault-server becomes ready in under 15 s on a 1M-note vault whose index is current, and a test proves a stale index is detected and rebuilt.

### 2026-10-12T15:56:13Z — wil

Status set to todo.

### 2026-10-12T15:56:52Z — /work

Claimed.

### 2026-10-12T16:54:00Z — wil

Verified set to verified.

### 2026-10-12T16:54:01Z — wil

Done on ticket/thevault-prd-38 (55303da). The store counts commits in .vault/generation (each manifest carries its generation and roll-forward writes it). index.db has a meta table holding the generation it is current with (index VERSION 2). rebuild sets it, and every writer commit site calls Index::advance(g), which moves only from g-1 to g, so a missed commit leaves the index stale and never wrongly current. A missing search/ dir also clears it. Writer::open skips both recover() and rebuild when the index is current: recover() was a second O(notes) pass, and a current index implies state.db finished every commit. No incremental catch-up: the store keeps no per-commit path log, so a stale index gets a full rebuild. Evidence: crates/writer/tests/startup.rs (current index trusted, shown by an out-of-band file it does not pick up; stale after a crash past the commit point, after a commit made without the writer, and after deleting index.db or search/, each rebuilt). S6 bench at 1M notes, seed 42: server ready in 0.3 s (was 1094.5 s). The same run's reindex took 2285.7 s because t37 ran its own 1M bench on the same disk at the same time; this change adds one statement to rebuild. Behaviour change: restarting the server no longer picks up out-of-band file edits (D28 makes them unsupported); vault reindex does.

### 2026-10-12T16:54:02Z — wil

Status set to done.

### 2026-10-12T17:09:51Z — wil

Merged into main as aa848c5 (fast-forward). Merged main (b6d940c) into the branch first, and ./dev just check exited 0 afterwards.
