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
status: todo
verified: unknown
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
