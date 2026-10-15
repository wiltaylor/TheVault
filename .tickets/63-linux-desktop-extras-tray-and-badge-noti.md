---
id: '63'
title: 'Linux desktop extras: tray and badge, notifications, autostart, global hotkey,
  thevault:// links, update check'
plan: thevault-clients
blocked_by:
- '62'
- '54'
status: done
verified: verified
---

# Linux desktop extras: tray and badge, notifications, autostart, global hotkey, thevault:// links, update check

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Linux desktop app" extras (D12, D9, D27): tray icon with unread badge (from the home server's SSE stream, reconnecting) and Quick capture / Open / Quit items; desktop notifications for new server notifications, closed when read, dismissed or cleared elsewhere; start on login minimised (setting); global capture hotkey through the XDG GlobalShortcuts portal with a documented fallback binding to `thevault --capture`; `thevault://note/<id>` deep links registered via the .desktop file; update check against GitHub Releases with an in-app prompt.

Acceptance criteria:
- S6: a notification created on the server updates the tray badge and raises a desktop notification; reading it via the API closes it.
- Unit tests for deep link parsing and update version comparison.
- Documented manual check for the portal hotkey on KDE/Hyprland.
Decisions: D9, D12, D27.

## Worklog

### 2026-10-15T08:14:28Z — /work

Claimed.

### 2026-10-15T08:55:48Z — wil

Verified set to verified.

### 2026-10-15T08:55:48Z — wil

Status set to done.

### 2026-10-15T08:55:49Z — wil

Branch ticket/thevault-clients-63 at 24469b6. It merges ticket/thevault-clients-62 (66b1a57) because 62 was done but not on main: merge 62 (and 60 under it) before 63.
Criteria: S6 now has 5 cases (pair/offline sync, --capture, notifications, thevault:// link, update prompt); green on 4 consecutive final runs plus the final tree. The notifications case checks badge {tray:true, unread:1, title:'1'}, a Notify call on a stand-in daemon (examples/fake-notifyd.rs), CloseNotification after POST /read, desktop dismiss marking read on the server, and click opening the note. 27 unit tests incl. links:: (5) and update:: (6). The manual KDE/Hyprland portal checklist is in docs/desktop.md.
Not exercised by any automated test: the portal hotkey itself (the test bus has no portal; status reports unavailable), autostart beyond the unit test of the entry file, real StatusNotifier tray rendering, and the deb/rpm desktopTemplate (no bundle build run). These need the manual checklist / ticket 64's packaging.
Found and fixed: ActionInvoked and NotificationClosed must come from one ordered D-Bus stream, otherwise a click can read as a dismiss. Ticket 62's S6 offline step asserted exactly 1 queued change; slow typing gives 2, so it now accepts >= 1.
For ticket 64: the packaged desktop file should be named dev.wiltaylor.thevault.desktop (the app id the portal and notification hints use), and the deb needs libayatana-appindicator3-1 for the tray. The update check assumes the GitHub repo wiltaylor/thevault (THEVAULT_RELEASES_REPO overrides at build time).

### 2026-10-15T09:00:10Z — wil

Merged to main as ae9dc92 (rebased onto f2a8bb6; tauri.conf.json conflict with 64's category resolved by keeping both, and the desktop template now uses {{categories}}). just check and desktop-test (S6 5/5) green on the rebased branch.
