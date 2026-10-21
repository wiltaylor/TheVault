---
id: '147'
title: Email and message notes have a processed checkbox, shown on their default and
  card templates, which sync never resets
blocked_by: []
status: todo
verified: unknown
---

# Email and message notes have a processed checkbox, shown on their default and card templates, which sync never resets

The user asked (2026-10-22) for a processed property on the email and message note types, to help decide what needs doing with emails and SMS as they come in.

Decisions (lead):
1. Field `processed`, kind `bool`, label "Processed", default false, not required, not computed, on the built-in `email` type (crates/writer/defaults/email.wcl) and `message` type (crates/writer/defaults/message.wcl), matching the `processed` field the built-in `fleeting` type already has (check its definition and mirror it, including any description wording).
2. Templates: add a `processed` slot to the default template of both types, beside the sender, and to the card template, so it can be ticked from a card in query views and kanban.
3. Existing vaults gain the field on their next start through the shipped-defaults seeding (crates/writer/src/shipped.rs adds missing fields to built-in types and a slot to the default template); no note is rewritten. Check that the card template slot is also added for existing vaults or document that only the default template gains it (that is the seeding rule today).
4. Queries: confirm `from email where !processed` and `from message where !processed` return notes that have no processed value (existing notes) as well as false ones; if a missing bool does not count as false in the query language, document the query to use (for example `!processed` vs `processed != true`) in the skill and docs.
5. Sync must never reset it: the Gmail task (crates/writer/defaults/google/google_gmail.ws) updates only labels on an existing email and must leave processed alone; the phone's device_messages.ws only creates message notes and must not touch processed on an existing note; the email and message types have no handler that writes it. Add tests: a processed email whose Gmail labels change stays processed; a message captured again (same device_ref) does not reset processed.
6. Docs: docs/vault-format.md (built-in types: email and message fields), and the agent skill crates/cli/skill/thevault/SKILL.md (built-in types list / a line telling an agent to set processed = true when it has dealt with an email or message, with the query for the unprocessed ones; its sh blocks run in a test, so any command added must succeed).
7. These are shipped built-in type files: if SHIPPED_FILES or any hash list covers them, update it; the shipped types tests (crates/server/tests/google_types.rs, phone_types.rs, crates/writer/tests/shipped_defaults.rs) must be updated for the new field.

Acceptance criteria:
- Type tests: email and message have processed (bool, default false) and the slots.
- Upgrade test: a vault with the 0.1.13-alpha.4 email and message types gains processed on start, no note changes, a second start does nothing.
- Query test for unprocessed notes including ones without a value.
- The two sync tests in point 5.
- ./dev just check and ./dev just android-test pass.

## Worklog
