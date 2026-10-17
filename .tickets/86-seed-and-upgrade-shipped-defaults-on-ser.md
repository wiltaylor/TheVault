---
id: '86'
title: 'Seed and upgrade shipped defaults on server start: missing built-in fields,
  device file and scripts, unedited shipped scripts, numbered upgrade steps'
blocked_by: []
status: done
verified: verified
---

# Seed and upgrade shipped defaults on server start: missing built-in fields, device file and scripts, unedited shipped scripts, numbered upgrade steps

Found on the live home server (vault made by `vault init --starter` before devices existed): `devices/` and `scripts/` were empty, so `import_history` and `upsert_contact` could not be enabled for the phone, and the vault's `person` type had no `phones`, `addresses` or `device_contacts`, so 160 of 174 contacts failed to import with "unknown field `phones`: person has no such field". Today `ensure_built_in_types` (crates/writer/src/builtin.rs:64-83) only adds a built-in type the vault lacks entirely, `vault init --starter` (crates/cli/src/starter.rs:81-87) is the only thing that writes the device file and device scripts, and nothing ever upgrades a shipped default once it is in a vault.

Build: on every server start (and in the embedded writer, crates/embedded/src/lib.rs:144-146), bring the vault's shipped defaults up to date, in one commit, recorded in history as made by the server:

1. Built-in types: add any type the vault lacks (as today), and add to an existing built-in type every field the shipped default declares that the vault's type lacks, as a planned schema change that touches no notes. Never remove, rename or change a field the user has, never touch templates the user has edited except to add a slot for an added field to the `default` template when that template exists.
2. Device file and device scripts: write `devices/android.wcl` and each shipped `scripts/device_*.ws` the vault lacks. Write scripts before the device file so nothing reports a false compile error.
3. Upgrading shipped scripts and device files: record, in state.db, the content hash of every shipped default written into the vault. On start, a vault file whose content still equals a hash the app shipped (any earlier version) is replaced by the current shipped version. A file the user changed is left alone, and the server adds one notification under the bell naming the file and saying a newer shipped version exists. Keep every shipped version's hash in the binary (a list per file) so a vault that skipped releases still upgrades.
4. Numbered upgrade steps: a small registry of one-time vault upgrades (id, function) applied in order and recorded in state.db as applied, for changes that are not additive, such as a field rename with a migration script. Ship the registry empty; ticket "Person contact fields" adds the first step.

Acceptance criteria:
- A vault created by the oldest starter (person with only email, organisation, homepage, body; no devices/ or scripts/) gains phones, addresses, device_contacts on server start, keeps organisation and homepage and every note unchanged, and gains devices/android.wcl plus the five device scripts with no compile problems reported.
- Starting the server twice changes nothing the second time (no new commit, no new history entry).
- A shipped script the user edited is not replaced, and exactly one notification names it; an unedited one from an earlier release is replaced.
- A user-deleted device script is re-added (state the behaviour in docs if you choose otherwise and say why in the worklog).
- An upgrade step runs once, is recorded, and is not run again on the next start.
- docs/vault-format.md documents the seeding and upgrade rules.
- `./dev just check` passes.

## Worklog

### 2026-10-17T20:46:28Z — /work

Claimed.

### 2026-10-17T21:01:13Z — wil

Verified set to verified.

### 2026-10-17T21:01:13Z — wil

Done on ticket/86 (ea89720). Code: crates/writer/src/shipped.rs (Writer::upgrade_shipped_defaults(Upkeep), SHIPPED_FILES hash lists, UPGRADE_STEPS registry: add a step with one line, see its doc comment), state.db v11 tables shipped_files and upgrades (crates/state/src/shipped.rs). Choices: steps run before seeding so a rename step sees the old field name; steps run only on the home server; the embedded writer (Upkeep::Device) seeds types and files but makes no notifications, since device notifications move to the server. A deleted device file or script is re-added on next start (documented). A shipped field the user removed from a built-in type is re-added on next start (documented: hide it instead); not tracked per field. A user-edited file with no record of what the server wrote is treated as based on an older version and notified once. Changing a shipped device file/script now requires appending its new hash (a unit test fails otherwise). testkit gained TestServer::restart. Tests that assumed a vault starts with no device file or scripts were adjusted (device_commands, completeness, phone_types, fleeting).

### 2026-10-17T21:01:13Z — wil

Status set to done.
