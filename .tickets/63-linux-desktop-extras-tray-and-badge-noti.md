---
id: '63'
title: 'Linux desktop extras: tray and badge, notifications, autostart, global hotkey,
  thevault:// links, update check'
plan: thevault-clients
blocked_by:
- '62'
- '54'
status: todo
verified: unknown
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
