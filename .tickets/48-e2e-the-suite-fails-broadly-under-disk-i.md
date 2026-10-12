---
id: '48'
title: 'e2e: the suite fails broadly under disk I/O pressure — write-path waits exceed
  the 5 s expect timeout'
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# e2e: the suite fails broadly under disk I/O pressure — write-path waits exceed the 5 s expect timeout

## Worklog

### 2026-10-12T19:05:11Z — wil

Found while working ticket 46 on 2026-10-13. ./dev just test-e2e on main at ce99147 (no changes) failed 37 of 65 tests; with the ticket-46 change it failed 15, 36 and 41 in three runs. Failures are write-path waits (create script toHaveURL, Save -> Edit button visible, restore -> URL) timing out at the default 5 s expect timeout. The same specs (admin, canvas, create-note) pass 15/15 with --workers=1. At the time /proc/pressure/io showed full avg10 ~25% (btrfs on dm-crypt; several fleet workers running), memory was fine. Likely environmental I/O contention combined with fsync on every write; options: lower e2e worker count, raise expect timeout in playwright.config.ts, or accept as environment. Needs a decision since it touches shared test config.

### 2026-10-12T19:24:28Z — wil

Triage (orchestrator): on an idle machine the suite passes 67/68 (the one failure is ticket 49, a real bug), so the broad failures are I/O contention. Fix: cap Playwright workers in playwright.config.ts (e.g. 4, overridable by E2E_WORKERS) and raise the expect timeout to 10 s. Do not weaken assertions. Acceptance: ./dev just test-e2e passes three runs in a row on main apart from ticket 49's test.

### 2026-10-12T19:24:28Z — wil

Status set to todo.

### 2026-10-12T19:33:34Z — /work

Claimed.

### 2026-10-12T19:42:21Z — wil

Verified set to verified.

### 2026-10-12T19:42:21Z — wil

Status set to done.

### 2026-10-12T19:42:21Z — wil

Fix in web/playwright.config.ts: workers default 4 (E2E_WORKERS overrides, positive integers only), expect.timeout 10_000. No assertions changed. ./dev just test-e2e passed 68/68 three runs in a row (4 workers each), with /proc/pressure/io full avg10 ~30% at the start of the runs; ticket 49's test also passed, so it looks fixed on main already. E2E_WORKERS=2 confirmed to run with 2 workers. ./dev just check exit 0.
