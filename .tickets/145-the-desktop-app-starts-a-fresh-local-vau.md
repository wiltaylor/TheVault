---
id: '145'
title: The desktop app starts a fresh local vault when its server holds another vault,
  as Android does
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# The desktop app starts a fresh local vault when its server holds another vault, as Android does

Ticket 143 made the sync engine record the home server's vault_id (GET /api/sync/info, and POST /api/devices/pair) in the local state.db and stop a round with sync::Error::OtherVault, before pushing, when the server names a different vault. The Android core (plugins/tauri-plugin-vault-device/src/core.rs, Core::start_fresh) answers it by moving the local vault aside to vault.prev-<ms> and opening a fresh one. The desktop app's sync loop (apps/desktop/src-tauri) does not: it shows the OtherVault error and stops syncing until the user deals with it, which is safe but leaves no way out in the app. Decide whether the desktop should start fresh the same way (and whether its pairing should carry vault_id), and build it. Also note: a pre-143 local vault that has synced (cursor or sync_notes) and is then paired by a new Android pairing starts fresh even when the server is the same one; its unsynced queued changes stay only in vault.prev-<ms> for 7 days.

## Worklog
