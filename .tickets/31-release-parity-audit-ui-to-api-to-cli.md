---
id: '31'
title: 'Release parity audit: UI to API to CLI'
plan: thevault-prd
blocked_by:
- '17'
- '18'
- '29'
- '21'
- '24'
- '25'
- '26'
- '27'
- '28'
- '30'
status: done
verified: verified
---

# Release parity audit: UI to API to CLI

Produce the sign-off checklist (D40).

Scope:
- `docs/parity.md`: every UI action, its API endpoint, its CLI command. Gaps are filed as `needs-triage` tickets.
- Run the agent-only check: a fresh agent session with only `vault skill` output imports a sample batch into a starter vault and fixes its own validation errors; record the transcript summary.
- Full `./dev just check` and `./dev just test-e2e` green on main.

Acceptance criteria:
- `docs/parity.md` lists every route in the server's endpoint listing and every UI action, with no unexplained gap.
- Every gap found is a filed ticket.
- The agent-only import succeeds, recorded in the worklog.
- Both test gates pass on main.

Seam: S1, S2, S5 and manual review. Decisions D32, D40.

## Worklog

### 2026-10-12T15:46:20Z — /work

Claimed.

### 2026-10-12T15:54:35Z — wil

Agent-only check (D32, D40): starter vault from 'vault init --starter', vault-server on port 0, a write-scoped token. A fresh general-purpose agent, told to learn TheVault only from 'vault skill --raw' and CLI output and to read no repo files, imported .scratch/agent-check/work/import.json (an old tracker's export: 2 people, 1 project, 3 tasks with deliberately bad values). It ran schema, then three staged batches (people, project, tasks), each dry-run first, using upserts keyed by email/title. The server returned 8 'invalid' errors, all fixed by the agent from the error JSON: homepage "tom's blog" is not an absolute URL (dropped, kept in body); status in-progress/done/waiting are not terms of taxonomy status (mapped to In progress/Done/Blocked); priority "urgent" is not one of low, medium, high (mapped to high); due 2026-11-31 and 'next friday' are not ISO 8601 dates (left empty, original in body). Field names were remapped (company->organisation, site->homepage, lead->owner, deadline->due, assigned_to->assignee). All 6 notes were created with references wired up, including the two_way project.tasks side. A repeat dry run reported unchanged x3. Read-back by query and get confirmed it. Verdict: succeeded with no outside help. Skill-doc gaps it reported are filed as ticket 45.

### 2026-10-12T15:54:35Z — wil

Gates: ./dev just check green on main at 3156cdf (242 Rust tests, 194 web) and on the branch after merging main bc2c5e6 (247 Rust, 194 web). ./dev just test-e2e: the first run on 3156cdf had one failure (shell.spec.ts:199, the 10000-note scroll, 5 s timeout), which passed 3/3 alone and 57/57 on a full rerun; it also passed 57/57 on the merged branch. Filed as 46. Parity audit gaps filed as 39-45. The UI inventory came from reading web/src. Map-pin 409 handling (44) and stale query views (43) are from code reading, not browser repros.

### 2026-10-12T15:54:35Z — wil

Verified set to verified.

### 2026-10-12T15:54:36Z — wil

Status set to done.
