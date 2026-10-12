---
id: '19'
title: 'Web shell: login, navigation, note view, PWA, conflicts'
plan: thevault-prd
blocked_by:
- 08
- '12'
status: done
verified: verified
---

# Web shell: login, navigation, note view, PWA, conflicts

Build the SolidJS client shell (D18, D19, D36).

Scope:
- Typed API client; login page; session handling.
- Navigation: types sidebar, note lists per type (virtualised), search box, open note.
- Note view rendering fields per the type's default template with tabs (read-only rendering here; editors come in later tickets).
- 409 conflict view showing your version beside the stored one (D12).
- PWA manifest and service worker that caches the app shell only; no offline data (D18).
- Responsive layout for tablet widths.

Acceptance criteria (S5 Playwright):
- Logging in with the right password reaches the app; the wrong one shows an error.
- A note created through the API appears in its type list and opens with its fields on the right tabs.
- Simulating a concurrent API write then saving from the UI shows the conflict view.
- Lighthouse-style check (or Playwright assertion) that the manifest and service worker register.
- A list of 10,000 notes scrolls with fewer than 200 rows in the DOM.

Seam: S5. Decisions D12, D14, D18, D19, D33, D36.

## Worklog

### 2026-10-12T13:15:30Z — /work

Claimed.

### 2026-10-12T13:26:01Z — wil

Verified set to verified.

### 2026-10-12T13:26:01Z — wil

Status set to done.

### 2026-10-12T13:26:01Z — wil

Committed ba08afb on ticket/thevault-prd-19. All five criteria proven by web/e2e/shell.spec.ts via ./dev just web-e2e-test (10 passed). Additions outside web/: GET /api/types (parsed type definitions incl. templates; ticket 16's schema inspection may supersede it — if 16 merges first, its endpoint-listing test needs /api/types added), vault-server --web <dir> (SPA fallback, /api never falls back), testkit-seed binary for S5 credentials. The note view's edit mode only covers title and single-valued scalar kinds; compound/multiple/reference fields are read-only until the editor tickets. web-e2e-test now builds vault-server and testkit-seed first.

### 2026-10-12T13:27:32Z — wil

Merged into main as f670c77 (fast-forward after merging main into the branch). Resolved conflicts in crates/api/src/lib.rs and types.rs by keeping main's type change/delete/flagged routes alongside GET /api/types. ./dev just check and web-e2e-test (10/10) pass on the merged tree.
