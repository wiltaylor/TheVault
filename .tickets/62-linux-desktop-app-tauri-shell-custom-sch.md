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
status: todo
verified: unknown
---

# Linux desktop app: Tauri shell, custom scheme to embedded server, setup and pairing, keyring, sync loop

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Linux desktop app" core (D2, D18, D21, D26, D27): apps/desktop Tauri 2 app that bundles web/dist, registers a custom URI scheme routed to crates/embedded `handle`, opens the local vault in the XDG data dir, first-run setup page (server URL plus password, or pair code) calling /api/devices/pair, stores the device token in the Secret Service keyring, runs the crates/sync loop (on start, on a timer, on network regain, after local writes) with all-assets policy, shows sync status and the "update the app" refusal. `thevault --capture` CLI flag opens the capture dialog (used by D12 fallback). S6 test with tauri-driver under Xvfb.

Acceptance criteria:
- S6: the app starts, pairs with a testkit server on port 0, shows notes, edits offline (server stopped), and the edit reaches the server after it restarts.
- `./dev just desktop-build` produces a binary; `./dev just desktop-test` runs S6 green.
Decisions: D2, D18, D19, D21, D26, D27.

## Worklog
