---
id: '144'
title: 'First sync shows progress: a download bar on the desktop setup page and in
  the sync popover with notes and files done, bytes and time left'
blocked_by: []
status: claimed
verified: unknown
---

# First sync shows progress: a download bar on the desktop setup page and in the sync popover with notes and files done, bytes and time left

The user asked (2026-10-21): when the desktop app is first paired with a server, show a download bar so it is clear how long the initial sync will take. Today the setup page runs the first sync with no progress (docs/desktop.md: the app "runs the first sync before it opens the web client").

Build:
1. Server: GET /api/sync/changes (and/or GET /api/sync/info) reports how far the change feed currently reaches (for example `latest` seq; check whether the feed already returns it) so a client can compute changes remaining from its cursor. GET /api/assets already lists sizes; confirm and use them.
2. Sync engine (crates/sync): a progress report updated during a round: phase (handshake, pushing, pulling notes, fetching files), notes done and total (from the cursor and `latest`), files done and total, bytes done and total for asset downloads (streamed since ticket 141, so count bytes as they arrive), and an ETA from the recent transfer rate. Exposed through the same status the sync popover reads (crates/embedded sync_status, GET /api/app/sync on devices) and to the desktop setup page.
3. Desktop setup page (apps/desktop): after pairing, the first sync shows a progress bar with "Notes N / M", "Files X / Y MB" and "about T left", updating at least once a second, and opens the web client when it finishes. A failure shows the error with Retry; closing the window leaves the sync to finish in the background on the next start.
4. The web sync popover (desktop and Android apps) shows the same progress during any round that pulls or downloads a lot (for example more than 200 changes or 20 MB), so the phone's first sync shows it too.
5. Docs: docs/desktop.md and docs/sync.md.

Acceptance criteria:
- crates/sync test against a test server holding many notes and assets: progress moves monotonically, totals are right, bytes match the asset sizes, the final report says done.
- Desktop: a test (unit or the existing S6 e2e harness in apps/desktop/e2e) shows the setup page's progress for a first sync against a server with enough data to take several seconds; screenshot to /home/wil/dev/TheVault/.scratch/shots/first-sync-progress.png.
- Web component test for the popover's progress line.
- ./dev just check passes (and the desktop tests the justfile runs for the app).

## Worklog

### 2026-10-21T10:39:54Z — wil

Claimed.
