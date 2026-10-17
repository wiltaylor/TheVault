---
id: '90'
title: 'Person contact fields: emails rename, every phone detail, two-way merge, upgrade
  step for existing vaults'
blocked_by:
- '86'
status: todo
verified: unknown
---

# Person contact fields: emails rename, every phone detail, two-way merge, upgrade step for existing vaults

The user decided (Q6 option 3, Q7 option 1, Q8 option 1) that phone contacts carry every detail the phone holds, both ways, and that the person fields are renamed so names match meanings. Today `person.addresses` holds email addresses (docs/scripts.md:348) and `person.email` is a separate single string the phone never fills.

The new person type (crates/writer/defaults/person.wcl), field names fixed:
- `emails` list of string: every email address (replaces `addresses` as email and the single `email`)
- `email_labels` list of string, index-aligned with `emails`
- `phones` list of string, E.164 (unchanged, keeps `has(phones, ...)` lookups working)
- `phone_labels` list of string, index-aligned with `phones`
- `addresses` list of string: postal addresses, each the phone's formatted address with line breaks joined by ", "
- `address_labels` list of string, index-aligned with `addresses`
- `organisation` string, `job_title` string, `nickname` string
- `birthday` string: "YYYY-MM-DD", or "--MM-DD" when the phone has no year
- `websites` list of url: a value without a scheme gets "https://"; one that still is not a URL is dropped
- `contact_note` string: the phone contact's note (kept apart from `body`, which is content)
- `photo` asset: the contact photo
- `device_contacts` dict, hidden (unchanged), `body` content (unchanged)
A label is "mobile", "home", "work", "main", "other", or the phone's custom label text.

Device payload contract (shared with the ticket "Android contacts read and write every detail"), in docs/scripts.md:
- `contact_changed` data: `{ contact_id, name, phones, phone_labels, emails, email_labels, addresses, address_labels, organisation, job_title, nickname, birthday, websites, contact_note, photo, updated, person, synced }`. `photo` is `{ asset, hash }` or null; the device stores the photo as an asset of the local vault before queuing, like MMS attachments, and `hash` is the SHA-256 of its bytes. `synced` carries the same keys except contact_id, updated, person, synced. A key missing from the payload (an older app) means "not reported": leave that person field alone.
- `upsert_contact` payload and `device::contacts_upsert`: the same detail keys plus `contact_id` and `person`; `photo` is an asset id or null.

Server side work:
1. person.wcl as above; crates/cli/starter/types/person.wcl the same plus the starter's own extras.
2. device_contacts.ws: the field-by-field merge (D16) covers every field; label lists move with their value lists as one field (phones+phone_labels, emails+email_labels, addresses+address_labels); photo compares by hash.
3. crates/writer/src/contacts.rs (queues upsert_contact when a person's contact fields change) and crates/script-core/src/device.rs: every new field.
4. The first numbered upgrade step from the ticket "Seed and upgrade shipped defaults on server start": for a vault whose person type has `addresses` and no `emails`, a planned type change with a migration script that renames `addresses` to `emails`, moves a non-empty `email` value into `emails` (first, de-duplicated) and removes the `email` field, and adds the new fields. Fields the user added (such as `homepage`) stay. Existing person notes keep every value.
5. Every other reader of person.addresses or person.email in crates/, web/ and docs (grep), and docs/vault-format.md, docs/scripts.md.

Acceptance criteria:
- An upgrade test from a vault with the 0.1.5 person type and people holding `email` and email `addresses`: after start every person has those values in `emails`, `email` is gone, `addresses` is empty, `homepage` kept.
- device_contacts tests: each new field merges as D16 says, a label list follows its values, an absent key leaves the field alone, photo compares by hash.
- upsert_contact queued from the writer carries every field (server test).
- `./dev just check` passes.

## Worklog
