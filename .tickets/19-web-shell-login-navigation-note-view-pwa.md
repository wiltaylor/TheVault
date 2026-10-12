---
id: '19'
title: 'Web shell: login, navigation, note view, PWA, conflicts'
plan: thevault-prd
blocked_by:
- 08
- '12'
status: todo
verified: unknown
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
