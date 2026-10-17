---
id: '89'
title: 'Android contacts read and write every detail: postal addresses, organisation,
  job title, birthday, websites, note, nickname, labels, photo'
blocked_by: []
status: done
verified: verified
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

### 2026-10-17T21:05:55Z — wil

Verified set to verified.

### 2026-10-17T21:05:55Z — wil

Android side done on ticket/89. Decisions ticket 90 must match: (1) synced.photo is {asset, hash}; hash is SHA-256 of the asset bytes written (the phone may re-encode, so the next read can show a new hash and a new asset once, then it is stable) - compare the contact side by hash vs synced.photo.hash and the person side by asset id vs synced.photo.asset. (2) synced only gains the keys an upsert carried; keys never written are absent (no base). organisation/job_title are agreed together. (3) Label list absent in an upsert keeps each value's existing label on the phone, else default mobile (phones), other (emails), home (addresses). Phone types other than mobile/home/work/main/other read as 'other'. (4) PERSON_MIME rows now carry DATA3='2'; older rows are rewritten once per process start before contacts are read (DeviceCapture.migrateSynced), bumping each linked contact's timestamp so each is re-captured once. (5) Until ticket 90 replaces device_contacts.ws, the shipped script still sends addresses meaning emails and maps payload addresses (now postal) to person.addresses: do not release main between 89 and 90. (6) New JNI VaultCore.assetBytes(id) / Core::asset_bytes fetches an evicted asset from the home server. Observer on Contacts.CONTENT_URI unchanged: the real provider notifies AUTHORITY_URI, which reaches descendant observers; Robolectric cannot model that, so change detection is tested via latestChange/changedSince.

### 2026-10-17T21:05:56Z — wil

Status set to done.
