---
id: '100'
title: A 0.1.6 device syncing a person type with the new labels/accept field items
  (ticket 93) may reject the type, since older schema parsers refuse unknown field
  items; decide whether vocab FORMAT_VERSION must rise or older readers need a tolerant
  path
blocked_by: []
status: done
verified: verified
---

# A 0.1.6 device syncing a person type with the new labels/accept field items (ticket 93) may reject the type, since older schema parsers refuse unknown field items; decide whether vocab FORMAT_VERSION must rise or older readers need a tolerant path

## Worklog

### 2026-10-17T23:11:23Z — wil

Decision (user, R1): readers tolerate field and type attributes they do not know instead of refusing the type, so a newer server never breaks an older app. Scope: (1) the schema parser reports an unknown attribute on a field or type as a warning-level issue that does not make the type invalid; the type editor and PUT /api/types still show it, so a typo is visible; a value of a known attribute that is malformed stays an error. (2) Find out and write in the worklog what a 0.1.6 reader (the embedded writer on the phone) does today when a synced type file has an unknown attribute: does sync stop, is only that type unusable, or are notes of the type refused; test it by running the 0.1.6 parsing path from git (tag or commit 5c85857) against the 0.1.7 person.wcl if practical. (3) docs/vault-format.md states the forward-compatibility rule; docs/android-release.md (or clients.md) states the upgrade order for a release that adds attributes: publish the new app first, let phones update while the old server still runs, then restart the server. Acceptance criteria: schema tests that an unknown field attribute and an unknown type attribute are warnings and the type still loads and validates notes; a test that the embedded writer opening a vault whose type has an unknown attribute still syncs and writes notes of that type; docs updated; ./dev just check and ./dev just android-test pass.

### 2026-10-17T23:11:23Z — wil

Status set to todo.

### 2026-10-17T23:11:51Z — /work

Claimed.

### 2026-10-17T23:29:31Z — wil

Verified set to verified.

### 2026-10-17T23:29:31Z — wil

Done on ticket/100 (1913cd6). Investigation, run at 5c85857 (0.1.6) with a probe test in crates/embedded against the 0.1.7 crates/writer/defaults/person.wcl: TypeDef::parse refuses it (field emails/phones/addresses.labels and photo.accept 'is not an item of this block', and because a field with an issue is dropped, every slot naming those fields fails too). During sync, apply_remote_files writes the file and returns Ok(false): sync does not stop, the whole in-memory schema stays the old one (no synced type change loads until the file is fixed), and person notes, including ones holding the new fields' data, are still created (201) against the stale person type; memo notes too. On the next start Embedded::open fails with 'loading the vault: the vault's type definitions are invalid: types/person.wcl ...', so a 0.1.6 app cannot open its vault at all. Hence the apps-first upgrade order in docs/android-release.md. Implementation: schema::Severity and Issue::warning; Check::attributes for type and field items; TypeDef.warnings; a refused type reports only its errors. An attribute that belongs to another kind (labels on a list of ints, accept on a string) is now also a warning and is not read. Left alone: unknown blocks, template/slot/handler items stay errors; POST /api/types/{name}/change plans carry no warnings; filed 105 (structural type.wcl schema stale) and 106 (editor save drops ignored attributes).

### 2026-10-17T23:29:31Z — wil

Status set to done.
