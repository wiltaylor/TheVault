---
id: '72'
title: 'web e2e: Playwright specs flake under host load (palette, paste-image, date-views
  kanban, placeholder-links, query-view and others time out at load avg ~25 with parallel
  workers)'
blocked_by: []
status: done
verified: verified
---

# web e2e: Playwright specs flake under host load (palette, paste-image, date-views kanban, placeholder-links, query-view and others time out at load avg ~25 with parallel workers)

## Worklog

### 2026-10-15T16:56:28Z — wil

Status set to claimed.

### 2026-10-15T16:56:28Z — wil

Root cause disk pressure (test vaults on the shared disk with fsync per write), not CPU. ./dev mounts /tmp as tmpfs; fixed two real app races it exposed (canvas caption/group label wiped by a save mid-typing; pins sidebar rebuilt every row so drags measured removed rows); paste-image waits for the caret; pins.spec measures visible rows; fleeting spec no longer races parallel captures. Proof: full e2e 4x259 under load avg 60-100 plus disk writers; full suite 262 passed on main.

### 2026-10-15T16:56:29Z — wil

Verified set to verified.

### 2026-10-15T16:56:29Z — wil

Status set to done.
