---
id: '89'
title: 'Android contacts read and write every detail: postal addresses, organisation,
  job title, birthday, websites, note, nickname, labels, photo'
blocked_by: []
status: claimed
verified: unknown
---

# Android contacts read and write every detail: postal addresses, organisation, job title, birthday, websites, note, nickname, labels, photo

The user decided (Q6 option 3, Q8 option 1) that the phone reads and writes every contact detail, both ways. Today ContactsReader.kt reads only the display name, phones (E.164, unnormalisable dropped) and emails, and writes only those.

Payload contract (shared with the ticket "Person contact fields: emails rename, every phone detail, two-way"), documented in docs/scripts.md by that ticket:
- `contact_changed` data: `{ contact_id, name, phones, phone_labels, emails, email_labels, addresses, address_labels, organisation, job_title, nickname, birthday, websites, contact_note, photo, updated, person, synced }`. `emails` replaces today's `addresses` (which held email). `addresses` is now postal: each StructuredPostal FORMATTED_ADDRESS with line breaks joined by ", ". Labels are index-aligned with their list: "mobile", "home", "work", "main", "other", or the custom LABEL text. `birthday` is "YYYY-MM-DD", or "--MM-DD" for a year-less Event.TYPE_BIRTHDAY. `websites` are strings as the phone holds them. `contact_note` is Note.NOTE. `photo` is `{ asset, hash }` or null: store the full-size photo (Photo / openContactPhotoInputStream, preferring the high-res one) as an asset of the local vault before the event is queued, like MMS attachments, and `hash` is the SHA-256 of the bytes; an unchanged hash reuses the earlier asset rather than storing it again. `synced` holds the same detail keys as last agreed.
- `upsert_contact` payload / `device::contacts_upsert(contact)`: the same detail keys plus `contact_id` and `person`; `photo` is an asset id of the local vault or null. Writing replaces each detail's rows on the contact's raw contacts (as phones and emails are replaced today), sets labels back as TYPE plus LABEL for custom ones, and writes the photo bytes from the asset. A key absent from the payload leaves those rows alone.
- Phones still normalise to E.164 and drop the ones that cannot be; their labels drop with them.

Also:
1. Stored `synced` state from an older app version uses `addresses` for emails: migrate it once on upgrade to `emails` so the first merge after the update does not treat every email as changed.
2. Change detection (contacts store observer, `latestChange`) must fire for edits to any of these details, not only name/phone/email.
3. Update FakePhone.kt, ReadersTest.kt and ContactSyncTest.kt for every new kind.

Acceptance criteria:
- Robolectric tests read each detail kind from FakePhone into the payload, including a custom label, a year-less birthday, and a photo stored once across two reads with the same bytes.
- Tests write each detail kind back through upsert_contact and read it again unchanged.
- The synced-state migration test.
- The Android unit tests and `./dev just check` pass.

## Worklog

### 2026-10-17T20:46:43Z — /work

Claimed.
