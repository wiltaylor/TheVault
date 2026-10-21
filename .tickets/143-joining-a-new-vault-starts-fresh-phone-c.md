---
id: '143'
title: 'Joining a new vault starts fresh: phone contact links from another vault are
  ignored, a change based on a version the server never had is pushed as new instead
  of stopping sync, and app backup is off'
blocked_by: []
status: done
verified: verified
---

# Joining a new vault starts fresh: phone contact links from another vault are ignored, a change based on a version the server never had is pushed as new instead of stopping sync, and app backup is off

Live bug, 2026-10-21. A phone (SM-F976B, app 0.1.13-alpha.2) was paired with a brand-new vault (https://vault.wil.dev, severus) after the user uninstalled, cleared data and reinstalled. After "Import from phone" the app queues 784 changes and no round ever completes: the sync popover shows "Last synced: Never" and "Last error: note 01M54NTRRY755C5BKDRZPEVDEC is gone from the server and its trash" (push_put's 404-with-base path in crates/sync/src/lib.rs returns Err, which ends the round).

Cause, confirmed with adb: the phone's contacts provider still holds 172 data rows of mimetype `vnd.android.cursor.item/vnd.dev.wiltaylor.thevault.person` (PERSON_MIME in ContactsReader.kt) written by contact sync against the user's previous dev vault; one of them is exactly 01M54NTRRY755C5BKDRZPEVDEC. These rows live in the system contacts database, so uninstall, clear data and reinstall leave them. On import, each linked contact arrives as contact_changed with `person` and `synced` from the old vault, so device_contacts.ws works on old person ids and the sync queue holds changes based on versions the new server never had. Separately the app manifest leaves android:allowBackup at its default (true): dumpsys shows ALLOW_BACKUP.

User decision: joining a new vault must start fresh.

Fix:
1. Vault-scoped contact links: the person link row stores the vault id (and the person id) it belongs to (for example data1 = person id, data2 = vault id, or a single value `<vault id>/<person id>`; keep reading the old format). ContactsReader reports `person` and `synced` only when the link's vault id matches the current local vault's server vault id; a link from another vault (or the old format with no vault id, when the person id does not exist in the local vault) is reported as unlinked, so device_contacts.ws creates or matches a person in the current vault as for a never-linked contact. With contacts-write on, the link row is rewritten for the current vault when the contact is linked; with it off, the stale row is left but ignored. Add a one-time cleanup on pairing with a new vault: rows linking to another vault are ignored (and removed when contacts-write is on).
2. Sync: a queued change whose base version the server never had and that is not in the server's trash (the 404 path where restore finds nothing) is pushed as a create under the note's id (If-None-Match: *) with the device's current content, not an error. Any note push that still fails (4xx other than 409 handling) is held back with its reason, the rest of the queue keeps pushing, and the popover's Not sent / Last error name it, as ticket 141 does for assets. The round still completes and records its time.
3. Pairing with a different vault starts fresh: the local vault records the server's vault id; pairing (or the first handshake) with a server whose vault id differs moves the local vault folder aside (vault.prev-<timestamp>, removed after 7 days or on the next successful sync, whichever is documented), clears the queue and cursors, and builds a fresh local vault from the server. The settings page says it did so.
4. Manifest: android:allowBackup="false", android:fullBackupContent="false" (or an empty rules file) and dataExtractionRules excluding everything for cloud backup and device transfer, so a reinstall never restores the vault, queue or device token.
5. Docs: docs/sync.md and docs/scripts.md (contact links are per vault).

Acceptance criteria:
- Robolectric: a contact with a link row from another vault id is read as unlinked; a link for the current vault is read as linked; the old-format row is handled as specified; contacts-write on rewrites the row.
- device_contacts tests (crates/embedded): importing contacts linked to another vault creates fresh persons in the current vault with new ids and queues only creates.
- crates/sync tests: a queued change with a base the server never had is pushed as a create; a note that the server refuses (for example 400) is held back while the others push and the round completes; status names it.
- A pairing test: pairing a device whose local vault came from server A with server B moves the old vault aside and syncs B's vault fresh.
- The built manifest has allowBackup false (check the merged manifest or aapt dump).
- ./dev just check and ./dev just android-test pass.

## Worklog

### 2026-10-21T10:04:13Z — wil

Claimed.

### 2026-10-21T10:43:15Z — wil

Evidence. Robolectric (just android-test, 141 tests, 0 failures): ReadersTest.aLinkRowFromAnotherVaultReadsAsUnlinked (other vault -> person/synced null; current vault -> linked; old format linked only when the person note exists locally; upsert rewrites the row with data4 = current vault and drops the other vault's synced), ReadersTest.forgettingOtherVaultsRemovesTheirLinksAndAdoptsOlderOnesWhosePersonIsHere, ContactSyncTest.aContactLinkedInAnotherVaultMakesAFreshPersonAndItsRowIsRewritten (real core, contacts_write on). crates/embedded tests/import.rs contacts_linked_to_another_vault_import_as_fresh_people_and_queue_only_creates. crates/sync tests/two_node.rs: a_change_based_on_a_version_the_server_never_had_is_pushed_as_a_create, a_note_the_server_refuses_is_held_back_while_the_others_push_and_the_round_completes, a_device_whose_local_vault_came_from_another_vault_stops_before_pushing; embedded sync_status a_note_the_server_refused_is_named_with_its_reason; web SyncStatus.test.tsx names the refused note. Pairing: plugin tests/core.rs pairing_with_another_vault_moves_the_local_vault_aside_and_syncs_the_new_one_fresh, a_handshake_with_another_vault_starts_fresh_and_syncs_it, a_local_vault_moved_aside_is_removed_after_seven_days. Merged manifest (app/build/intermediates/merged_manifests/universalDebug) has allowBackup=false, fullBackupContent=false, dataExtractionRules=@xml/data_extraction_rules. ./dev just check and ./dev just android-test exit 0. device_contacts.ws unchanged: the reader reports foreign links as unlinked, so the script's never-linked path already matches or creates; no SHIPPED_FILES hash added. Filed 145 (desktop fresh start).

### 2026-10-21T10:43:15Z — wil

Verified set to verified.

### 2026-10-21T10:43:36Z — wil

Status set to done.
