---
id: '145'
title: The desktop app starts a fresh local vault when its server holds another vault,
  as Android does
blocked_by: []
status: done
verified: verified
---

# The desktop app starts a fresh local vault when its server holds another vault, as Android does

Ticket 143 made the sync engine record the home server's vault_id (GET /api/sync/info, and POST /api/devices/pair) in the local state.db and stop a round with sync::Error::OtherVault, before pushing, when the server names a different vault. The Android core (plugins/tauri-plugin-vault-device/src/core.rs, Core::start_fresh) answers it by moving the local vault aside to vault.prev-<ms> and opening a fresh one. The desktop app's sync loop (apps/desktop/src-tauri) does not: it shows the OtherVault error and stops syncing until the user deals with it, which is safe but leaves no way out in the app. Decide whether the desktop should start fresh the same way (and whether its pairing should carry vault_id), and build it. Also note: a pre-143 local vault that has synced (cursor or sync_notes) and is then paired by a new Android pairing starts fresh even when the server is the same one; its unsynced queued changes stay only in vault.prev-<ms> for 7 days.

## Worklog

### 2026-10-21T20:22:30Z — wil

Status set to todo.

### 2026-10-21T20:22:30Z — wil

Decided (user, 2026-10-22): yes, the desktop starts fresh exactly as Android does. Its pairing records vault_id (POST /api/devices/pair answers it) in device.json and the local state.db; a round that meets sync::Error::OtherVault, or a pairing with a server whose vault_id differs from the local vault's recorded one, moves ~/.local/share/thevault/vault aside to vault.prev-<ms> beside it, opens a fresh local vault, pulls the server's vault whole (with ticket 144's progress on the setup page or the sync popover), and removes vault.prev-* older than 7 days on start or after a round. The app shows a one-line notice (Settings > Devices, like Android's fresh_start in GET /api/app) saying it started fresh and where the old copy went. Reuse the Android logic (Core::start_fresh) by moving it into a shared place (crates/embedded or crates/sync) rather than duplicating it. Acceptance: a desktop test (Rust unit or the apps/desktop/e2e harness) where a desktop vault synced from server A is pointed at server B (re-pair) and at a server whose vault changed under the same pairing: both start fresh, keep the old folder, sync B, and show the notice; Android's existing tests still pass; docs/desktop.md and docs/sync.md updated; ./dev just check, ./dev just android-test and the desktop e2e pass.

### 2026-10-21T20:22:50Z — wil

Claimed.

### 2026-10-21T20:52:26Z — wil

Verified set to verified.

### 2026-10-21T20:52:31Z — wil

Status set to done.
