---
id: '150'
title: A device pushes the notes its changes link to, and once adopts local notes
  the server never had, so leftovers from an earlier vault upload instead of blocking
  435 messages; plus a Send this device's notes button
blocked_by: []
status: done
verified: verified
---

# A device pushes the notes its changes link to, and once adopts local notes the server never had, so leftovers from an earlier vault upload instead of blocking 435 messages; plus a Send this device's notes button

Live bug, 2026-10-22, the user's phone (SM-F976B, app 0.1.14-alpha.1) on vault.wil.dev (severus, 0.1.14-alpha.1). Every round ends "pushed 0 ... notes held 435: 435 notes could not be sent to the server: note 01M5EK8P7NP7T5SD5WBY1JJ2A6: writing note ...: the server answered 400: {"error":"invalid","file":"notes/message/01M5EK8P7NP7T5SD5WBY1JJ2A6.wcl","issues":[{"at":"conversation","message":"note 01M54QB0DV70S5SPH7H199H2Q9 does not exist"}]}" (and 434 more).

Cause: the phone was paired with severus before ticket 143 (vault ids), so on its first 0.1.13-alpha.3 sync it recorded severus's vault id instead of starting fresh, while its local vault still held notes pulled from the user's previous dev vault (conversations 01M54Q..., inbox fleeting notes, and maybe others). Ticket 143 pushes queued changes the server never had, but pulled notes were never queued, so they never reached severus. New messages from "Import from phone" link to those local-only conversations and the server refuses them. The user's inbox on the phone holds notes they want to keep that the server does not have (severus has 1 fleeting note), so clearing app data is not acceptable.

Fix:
1. Dependencies first: when pushing a note fails with a 400 whose issues say a referenced note does not exist (a reference field, a conversation, a mention if the server validates those), and the device holds that note locally, push the referenced note first (as a create, recursively, with a cycle guard), then retry the original in the same round. Prefer a structured error code from the server over parsing the message: if the server's invalid answer does not carry the missing note id in a field, add one (for example issues[].missing_note).
2. Adopt local-only notes once: a local vault that recorded its vault id at a first sync (not at pairing), or any local vault on demand, compares its note ids with the server's (use the change feed from 0, or add a light endpoint listing note ids and versions if the feed is too heavy) and queues every local note the server lacks as a create with no base. Record that the adoption ran so it does not repeat. Trashed or archived local notes follow their state.
3. Settings > Devices > This device gets "Send this device's notes to the server", which runs step 2 now and reports how many notes it queued; sync then pushes them with progress (ticket 144).
4. Docs: docs/sync.md.

Acceptance criteria:
- crates/sync test: a device whose local vault holds a conversation the server lacks pushes a message linking to it: the conversation is pushed first and both arrive; a cycle of two notes linking each other both arrive.
- crates/sync or plugin test: a local vault from a pre-143 pairing (vault id recorded at first sync) with local-only notes queues them once and they arrive; a second run queues nothing; a note the server already has is not re-sent.
- The This device button: a web component test and a core test of the endpoint it calls.
- ./dev just check and ./dev just android-test pass.
- Do not wipe or rewrite any local note; only queue creates.

## Worklog

### 2026-10-22T13:40:30Z — wil

Claimed.

### 2026-10-22T14:04:35Z — wil

Verified set to verified.

### 2026-10-22T14:04:36Z — wil

Implemented on ticket/150 (971efa5). Tests: sync two_node (conversation pushed first, two-note cycle, pre-143 adoption once incl. archived/trashed, endpoint), server notes missing_note, plugin core endpoint test, web SendDeviceNotes test. ./dev just check and ./dev just android-test pass. Not yet confirmed on the user's phone.

### 2026-10-22T14:04:36Z — wil

Status set to done.
