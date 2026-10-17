---
id: '100'
title: A 0.1.6 device syncing a person type with the new labels/accept field items
  (ticket 93) may reject the type, since older schema parsers refuse unknown field
  items; decide whether vocab FORMAT_VERSION must rise or older readers need a tolerant
  path
blocked_by: []
status: todo
verified: unknown
---

# A 0.1.6 device syncing a person type with the new labels/accept field items (ticket 93) may reject the type, since older schema parsers refuse unknown field items; decide whether vocab FORMAT_VERSION must rise or older readers need a tolerant path

## Worklog

### 2026-10-17T23:11:23Z — wil

Decision (user, R1): readers tolerate field and type attributes they do not know instead of refusing the type, so a newer server never breaks an older app. Scope: (1) the schema parser reports an unknown attribute on a field or type as a warning-level issue that does not make the type invalid; the type editor and PUT /api/types still show it, so a typo is visible; a value of a known attribute that is malformed stays an error. (2) Find out and write in the worklog what a 0.1.6 reader (the embedded writer on the phone) does today when a synced type file has an unknown attribute: does sync stop, is only that type unusable, or are notes of the type refused; test it by running the 0.1.6 parsing path from git (tag or commit 5c85857) against the 0.1.7 person.wcl if practical. (3) docs/vault-format.md states the forward-compatibility rule; docs/android-release.md (or clients.md) states the upgrade order for a release that adds attributes: publish the new app first, let phones update while the old server still runs, then restart the server. Acceptance criteria: schema tests that an unknown field attribute and an unknown type attribute are warnings and the type still loads and validates notes; a test that the embedded writer opening a vault whose type has an unknown attribute still syncs and writes notes of that type; docs updated; ./dev just check and ./dev just android-test pass.

### 2026-10-17T23:11:23Z — wil

Status set to todo.
