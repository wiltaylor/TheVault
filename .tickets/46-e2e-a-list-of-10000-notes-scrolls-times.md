---
id: '46'
title: 'e2e: ''a list of 10000 notes scrolls'' times out under parallel load'
plan: thevault-prd
blocked_by: []
status: done
verified: unmet
---

# e2e: 'a list of 10000 notes scrolls' times out under parallel load

Found while running ./dev just test-e2e for ticket 31 on main at 3156cdf. web/e2e/shell.spec.ts:199 waited 5 s for the 'Bulk 10000' row after scrolling to the bottom and found none; 56 other tests passed. Alone it passed three times out of three (about 9 s each), and the full suite passed (57/57) on a rerun. It looks like the default 5 s expect timeout is too short for the last page to load when other workers are loading the machine.

## Worklog

### 2026-10-12T15:56:14Z — wil

Triage (orchestrator): Fix: make the 10,000-row scroll test robust under parallel load (longer timeout or serial project) without weakening the under-200-rows assertion. Acceptance: ./dev just test-e2e passes three runs in a row.

### 2026-10-12T15:56:14Z — wil

Status set to todo.

### 2026-10-12T18:50:02Z — /work

Claimed.

### 2026-10-12T19:05:11Z — wil

Verified set to unmet.

### 2026-10-12T19:05:11Z — wil

Fix on ticket/thevault-prd-46 (d35b15a): test.slow() and 20 s timeouts on the visibility waits in shell.spec.ts 'a list of 10000 notes scrolls'; the <200 row assertions unchanged. The test passed in all three full runs (15.4 s, 17.5 s, 25.2 s; 25 s is near the default 30 s test timeout, which test.slow() triples). Criterion './dev just test-e2e passes three runs in a row' UNMET: every run failed on other tests (15/36/41 failures) because disk I/O was saturated (PSI io full ~25%); baseline main without this change also failed 37/65. Filed as ticket 48 (needs-triage). ./dev just check passes.

### 2026-10-12T19:05:11Z — wil

Status set to done.

### 2026-10-12T19:15:14Z — wil

Merged into main at e42c704 (fast-forward after merging main into the branch; ./dev just check passed post-merge).
