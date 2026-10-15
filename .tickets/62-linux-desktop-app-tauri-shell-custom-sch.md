---
id: '62'
title: 'Linux desktop app: Tauri shell, custom scheme to embedded server, setup and
  pairing, keyring, sync loop'
plan: thevault-clients
blocked_by:
- '51'
- '53'
- '59'
- '60'
status: done
verified: verified
---

# Linux desktop app: Tauri shell, custom scheme to embedded server, setup and pairing, keyring, sync loop

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Linux desktop app" core (D2, D18, D21, D26, D27): apps/desktop Tauri 2 app that bundles web/dist, registers a custom URI scheme routed to crates/embedded `handle`, opens the local vault in the XDG data dir, first-run setup page (server URL plus password, or pair code) calling /api/devices/pair, stores the device token in the Secret Service keyring, runs the crates/sync loop (on start, on a timer, on network regain, after local writes) with all-assets policy, shows sync status and the "update the app" refusal. `thevault --capture` CLI flag opens the capture dialog (used by D12 fallback). S6 test with tauri-driver under Xvfb.

Acceptance criteria:
- S6: the app starts, pairs with a testkit server on port 0, shows notes, edits offline (server stopped), and the edit reaches the server after it restarts.
- `./dev just desktop-build` produces a binary; `./dev just desktop-test` runs S6 green.
Decisions: D2, D18, D19, D21, D26, D27.

## Worklog

### 2026-10-15T07:50:17Z — /work

Claimed.

### 2026-10-15T08:14:07Z — wil

Verified set to verified.

### 2026-10-15T08:14:07Z — wil

Status set to done.

### 2026-10-15T08:14:07Z — wil

Branch ticket/thevault-clients-62 at 66b1a57 sits on top of ticket 60's 261f4da (crates/sync was not on main when claimed): merge 60 first. S6 green 7/7 runs after fixing one test bug (the web client sends / on to /daily). Not covered here, left for 63: tray, badge, notifications, autostart, portal hotkey, thevault:// links, update check. The single-instance path of --capture (forwarding to a running app) is wired but not driven by S6; S6 covers --capture on a fresh launch. Network regain is detected by probing /api/sync/info every 3 s while offline, not through NetworkManager. The dev image gains gnome-keyring and libnss-wrapper (dbus-daemon refuses the passwd-less ./dev uid).
