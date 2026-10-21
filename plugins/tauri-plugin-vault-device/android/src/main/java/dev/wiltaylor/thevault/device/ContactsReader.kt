package dev.wiltaylor.thevault.device

import android.content.ContentProviderOperation
import android.content.ContentResolver
import android.content.ContentUris
import android.content.SharedPreferences
import android.net.Uri
import android.provider.ContactsContract
import android.provider.ContactsContract.CommonDataKinds.BaseTypes
import android.provider.ContactsContract.CommonDataKinds.Email
import android.provider.ContactsContract.CommonDataKinds.Event
import android.provider.ContactsContract.CommonDataKinds.Nickname
import android.provider.ContactsContract.CommonDataKinds.Note
import android.provider.ContactsContract.CommonDataKinds.Organization
import android.provider.ContactsContract.CommonDataKinds.Phone
import android.provider.ContactsContract.CommonDataKinds.Photo
import android.provider.ContactsContract.CommonDataKinds.StructuredName
import android.provider.ContactsContract.CommonDataKinds.StructuredPostal
import android.provider.ContactsContract.CommonDataKinds.Website
import android.provider.ContactsContract.Contacts
import android.provider.ContactsContract.Data
import android.provider.ContactsContract.DeletedContacts
import android.provider.ContactsContract.PhoneLookup
import android.provider.ContactsContract.RawContacts
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import java.io.IOException
import java.security.MessageDigest

/** Finds the phone contact a number belongs to, as a payload's `contact`: `{ id, name }`. */
class ContactLookup(private val resolver: ContentResolver) {
    fun find(raw: String?): JSONObject? {
        if (raw.isNullOrBlank()) return null
        return try {
            val uri = Uri.withAppendedPath(PhoneLookup.CONTENT_FILTER_URI, Uri.encode(raw))
            resolver.rows(uri, arrayOf(PhoneLookup._ID, PhoneLookup.DISPLAY_NAME), limit = 1) { c ->
                JSONObject()
                    .put("id", c.long(PhoneLookup._ID).toString())
                    .put("name", c.string(PhoneLookup.DISPLAY_NAME) ?: "")
            }.firstOrNull()
        } catch (_: SecurityException) {
            // Without READ_CONTACTS a number still arrives, just with no contact.
            null
        }
    }
}

/**
 * Reads, adds, changes and deletes phone contacts through `ContactsContract`, as
 * `contact_changed` and `contact_deleted` payloads (`docs/scripts.md`): `{ contact_id, name,
 * phones, phone_labels, emails, email_labels, addresses, address_labels, organisation, job_title,
 * nickname, birthday, websites, contact_note, photo, updated, person, synced }` (D16).
 *
 * - `phones` are E.164; a number that does not normalise is dropped, with its label.
 * - `addresses` are postal addresses, each the phone's formatted address with its lines joined
 *   by ", ".
 * - A label list is index-aligned with its values: "mobile", "home", "work", "main", "other",
 *   or the phone's custom label text.
 * - `birthday` is "YYYY-MM-DD", or "--MM-DD" for a birthday with no year; "" for none.
 * - `photo` is `{ asset, hash }` or null: the full-size photo, stored as an asset of the local
 *   vault before the event is queued ([ContactPhotos]), and the SHA-256 of its bytes.
 *
 * A contact linked to a `person` note carries one data row of its own, [PERSON_MIME], holding
 * the note's id and what the contact and the person last agreed on (D16, D24): `person` and
 * `synced` in the payload, both null on a contact never linked. The contacts app does not show
 * the row. [SYNCED_FORMAT] in its `DATA3` marks the `synced` written by this version: an older
 * one used `addresses` for the email addresses, which [migrateSynced] renames to `emails`.
 *
 * A link belongs to one vault (ticket 143): `DATA4` holds the id of the home server's vault the
 * person is in. The row lives in the phone's contacts database, so it outlives the app being
 * uninstalled, its data cleared and the phone paired with another vault. A link is reported only
 * when its vault is the one the local vault is a copy of ([ContactLinks.vaultId]); a link from
 * another vault is reported as never linked, so the device file matches or creates a person in
 * this vault. A row an older version wrote names no vault: it counts as this vault's when the
 * local vault holds its person ([ContactLinks.personExists]), and as never linked otherwise.
 * [upsert] writes the row for this vault; [forgetOtherVaults] removes those of other vaults.
 *
 * A changed contact's key is its last-updated time, and a deleted one's its deletion time, in
 * milliseconds: the provider gives no other order for changes. The provider moves that time for
 * an edit to any data row of the contact, so every detail above is change-detected.
 */
class ContactsReader(
    private val resolver: ContentResolver,
    private val numbers: PhoneNumbers,
    private val photos: ContactPhotos,
    private val links: ContactLinks = ContactLinks(),
) {
    private val columns = arrayOf(
        Contacts._ID,
        Contacts.DISPLAY_NAME,
        Contacts.CONTACT_LAST_UPDATED_TIMESTAMP,
    )

    /** Contacts changed after [since], oldest change first, up to [until]. */
    fun changedSince(since: Long, until: Long = Long.MAX_VALUE): List<Captured> =
        resolver.rows(
            Contacts.CONTENT_URI,
            columns,
            "${Contacts.CONTACT_LAST_UPDATED_TIMESTAMP} > ? AND ${Contacts.CONTACT_LAST_UPDATED_TIMESTAMP} <= ?",
            arrayOf(since.toString(), until.toString()),
            "${Contacts.CONTACT_LAST_UPDATED_TIMESTAMP} ASC",
        ) { c -> Triple(c.long(Contacts._ID), c.string(Contacts.DISPLAY_NAME), c.long(Contacts.CONTACT_LAST_UPDATED_TIMESTAMP)) }
            .map { (id, name, updated) -> Captured("contact_changed", contact(id, name, updated), updated, updated) }

    /** Contacts deleted after [since], oldest first. */
    fun deletedSince(since: Long): List<Captured> =
        resolver.rows(
            DeletedContacts.CONTENT_URI,
            arrayOf(DeletedContacts.CONTACT_ID, DeletedContacts.CONTACT_DELETED_TIMESTAMP),
            "${DeletedContacts.CONTACT_DELETED_TIMESTAMP} > ?",
            arrayOf(since.toString()),
            "${DeletedContacts.CONTACT_DELETED_TIMESTAMP} ASC",
        ) { c ->
            val deleted = c.long(DeletedContacts.CONTACT_DELETED_TIMESTAMP)
            val data = JSONObject().put("contact_id", c.long(DeletedContacts.CONTACT_ID).toString())
            Captured("contact_deleted", data, deleted, deleted)
        }

    /** The newest change time, where a cursor starts so that nothing earlier is captured. */
    fun latestChange(): Long = resolver.rows(
        Contacts.CONTENT_URI,
        arrayOf(Contacts.CONTACT_LAST_UPDATED_TIMESTAMP),
        order = "${Contacts.CONTACT_LAST_UPDATED_TIMESTAMP} DESC",
        limit = 1,
    ) { it.long(Contacts.CONTACT_LAST_UPDATED_TIMESTAMP) }.firstOrNull() ?: 0L

    /** The newest deletion time. */
    fun latestDeletion(): Long = resolver.rows(
        DeletedContacts.CONTENT_URI,
        arrayOf(DeletedContacts.CONTACT_DELETED_TIMESTAMP),
        order = "${DeletedContacts.CONTACT_DELETED_TIMESTAMP} DESC",
        limit = 1,
    ) { it.long(DeletedContacts.CONTACT_DELETED_TIMESTAMP) }.firstOrNull() ?: 0L

    /** `device::contacts_list`: `name` is an exact display name, `number` an E.164 number. */
    fun list(filter: JSONObject?): List<JSONObject> {
        val name = filter.text("name")
        val number = filter.text("number")
        val all = resolver.rows(
            Contacts.CONTENT_URI,
            columns,
            name?.let { "${Contacts.DISPLAY_NAME} = ?" },
            name?.let { arrayOf(it) },
            "${Contacts.DISPLAY_NAME} ASC",
        ) { c -> Triple(c.long(Contacts._ID), c.string(Contacts.DISPLAY_NAME), c.long(Contacts.CONTACT_LAST_UPDATED_TIMESTAMP)) }
        return all.asSequence()
            .map { (id, name, updated) -> id to read(id, name, updated) }
            .filter { (_, details) -> number == null || details.phones.values.contains(number) }
            .take(filter.limit())
            .map { (id, details) -> payload(id, details, store = true) }
            .toList()
    }

    /** `device::contacts_get`. */
    fun get(contactId: String): JSONObject {
        val (id, name, updated) = find(contactId)
        return contact(id, name, updated)
    }

    /**
     * `device::contacts_upsert`: add the contact, or change contact `contact_id`, to hold the
     * details given: any of `name`, `phones` and `phone_labels`, `emails` and `email_labels`,
     * `addresses` and `address_labels`, `organisation`, `job_title`, `nickname`, `birthday`,
     * `websites`, `contact_note` and `photo` (an asset id of the local vault, or null). A key left
     * out leaves that detail alone; a label list left out keeps each value's label on the phone.
     * Only the details that differ are rewritten, so a contact that already agrees keeps its own
     * number formatting. With `person`, the contact's [PERSON_MIME] row is set to the note's id
     * and to the details as what the two sides now agree on, and a contact with no `contact_id`
     * that already holds that person is changed rather than added twice. Returns `{ contact_id }`.
     */
    fun upsert(contact: JSONObject): JSONObject {
        val person = contact.text("person")
        val existing = contact.text("contact_id") ?: person?.let(::contactOfPerson)
        val found = existing?.let(::find)
        val now = found?.let { (id, name, updated) -> read(id, name, updated) } ?: Details.EMPTY
        val vault = links.vaultId()
        val row = found?.let { personRow(it.first) }
        // What another vault's person agreed with the contact means nothing to this one's.
        val held = row?.takeIf { belongs(it, vault) }
        val want = wanted(contact, now)

        val ops = ArrayList<ContentProviderOperation>()
        val raws = existing?.let(::rawContactsOf)
        val rawId = raws?.first()
        if (existing == null) {
            ops.add(
                ContentProviderOperation.newInsert(RawContacts.CONTENT_URI)
                    .withValue(RawContacts.ACCOUNT_TYPE, null)
                    .withValue(RawContacts.ACCOUNT_NAME, null)
                    .build(),
            )
        }
        fun replace(mime: String, extra: Pair<String, Array<String>>? = null, add: () -> Unit) {
            if (raws != null) ops.add(deleteRows(raws, mime, extra))
            add()
        }
        if (want.name != null && want.name != now.name) {
            replace(StructuredName.CONTENT_ITEM_TYPE) {
                ops.add(row(rawId, StructuredName.CONTENT_ITEM_TYPE).withValue(StructuredName.DISPLAY_NAME, want.name).build())
            }
        }
        for (kind in LIST_KINDS) {
            val values = want.lists[kind.key] ?: continue
            if (raws != null && values == now.lists.getValue(kind.key)) continue
            replace(kind.mime) {
                values.values.forEachIndexed { i, value ->
                    val label = values.labels[i]
                    val type = kind.typeOf(label)
                    val b = row(rawId, kind.mime).withValue(kind.valueColumn, value).withValue(TYPE, type)
                    if (type == BaseTypes.TYPE_CUSTOM) b.withValue(LABEL, label)
                    ops.add(b.build())
                }
            }
        }
        if ((want.organisation != null || want.jobTitle != null) &&
            (want.organisation ?: now.organisation) to (want.jobTitle ?: now.jobTitle) != now.organisation to now.jobTitle
        ) {
            val company = want.organisation ?: now.organisation
            val title = want.jobTitle ?: now.jobTitle
            replace(Organization.CONTENT_ITEM_TYPE) {
                if (company.isNotEmpty() || title.isNotEmpty()) {
                    ops.add(
                        row(rawId, Organization.CONTENT_ITEM_TYPE)
                            .withValue(Organization.COMPANY, company.ifEmpty { null })
                            .withValue(Organization.TITLE, title.ifEmpty { null })
                            .withValue(Organization.TYPE, Organization.TYPE_WORK)
                            .build(),
                    )
                }
            }
        }
        fun single(value: String?, current: String, mime: String, column: String, extra: Pair<String, Array<String>>? = null, more: (ContentProviderOperation.Builder) -> Unit = {}) {
            if (value == null || value == current) return
            replace(mime, extra) {
                if (value.isNotEmpty()) ops.add(row(rawId, mime).withValue(column, value).also(more).build())
            }
        }
        single(want.nickname, now.nickname, Nickname.CONTENT_ITEM_TYPE, Nickname.NAME)
        single(want.note, now.note, Note.CONTENT_ITEM_TYPE, Note.NOTE)
        single(
            want.birthday, now.birthday, Event.CONTENT_ITEM_TYPE, Event.START_DATE,
            "${Event.TYPE} = ?" to arrayOf(Event.TYPE_BIRTHDAY.toString()),
        ) { it.withValue(Event.TYPE, Event.TYPE_BIRTHDAY) }
        if (want.websites != null && want.websites != now.websites) {
            replace(Website.CONTENT_ITEM_TYPE) {
                want.websites.forEach {
                    ops.add(row(rawId, Website.CONTENT_ITEM_TYPE).withValue(Website.URL, it).withValue(Website.TYPE, Website.TYPE_OTHER).build())
                }
            }
        }
        // The photo: null clears it, an asset id is written from the asset's bytes after the batch.
        var photoBytes: ByteArray? = null
        var photo: JSONObject? = now.photo?.let { photos.known(it) }
        var photoAgreed = want.photo
        if (contact.has("photo")) {
            val asset = contact.text("photo")
            if (asset == null) {
                if (now.photo != null && raws != null) ops.add(deleteRows(raws, Photo.CONTENT_ITEM_TYPE))
                photo = null
            } else if (now.photo == null || photos.assetOf(now.photo.hash) != asset) {
                val bytes = photos.bytes(asset)
                if (bytes == null) {
                    Log.w(TAG, "photo asset $asset is not on this device; the contact keeps its photo")
                    photoAgreed = false
                } else {
                    val hash = ContactPhotos.sha256(bytes)
                    photos.remember(hash, asset)
                    photo = JSONObject().put("asset", asset).put("hash", hash)
                    if (now.photo?.hash != hash) {
                        if (raws != null && now.photo != null) ops.add(deleteRows(raws, Photo.CONTENT_ITEM_TYPE))
                        photoBytes = bytes
                    }
                }
            }
        }

        var contactId = existing
        var firstRaw = rawId
        if (ops.isNotEmpty()) {
            val results = resolver.applyBatch(ContactsContract.AUTHORITY, ops)
            if (existing == null) {
                firstRaw = results.firstOrNull()?.uri?.let { ContentUris.parseId(it) }
                    ?: throw IllegalStateException("the phone did not add the contact")
                contactId = resolver.rows(
                    RawContacts.CONTENT_URI,
                    arrayOf(RawContacts.CONTACT_ID),
                    "${RawContacts._ID} = ?",
                    arrayOf(firstRaw.toString()),
                    limit = 1,
                ) { it.long(RawContacts.CONTACT_ID) }.firstOrNull()?.toString()
                    ?: throw IllegalStateException("the phone added the contact but has no id for it")
            }
        }
        if (photoBytes != null) writePhoto(firstRaw!!, photoBytes)

        if (person != null) {
            val synced = upgraded(held?.synced, held?.format) ?: JSONObject()
            want.copy(photo = photoAgreed).agreed(now, photo).forEach { (key, value) -> synced.put(key, value) }
            if (held?.person != person || held.synced?.toString() != synced.toString() || held.format != SYNCED_FORMAT || row?.vault != vault) {
                val raw = firstRaw!!
                val personOps = ArrayList<ContentProviderOperation>()
                rawContactsOf(contactId!!).let { personOps.add(deleteRows(it, PERSON_MIME)) }
                personOps.add(
                    row(raw, PERSON_MIME)
                        .withValue(Data.DATA1, person)
                        .withValue(Data.DATA2, synced.toString())
                        .withValue(Data.DATA3, SYNCED_FORMAT)
                        .withValue(VAULT, vault)
                        .build(),
                )
                resolver.applyBatch(ContactsContract.AUTHORITY, personOps)
            }
        }
        return JSONObject().put("contact_id", contactId)
    }

    /** `device::contacts_delete`. */
    fun delete(contactId: String) {
        val id = contactId.toLongOrNull() ?: throw IllegalArgumentException("$contactId is not a contact id")
        resolver.delete(ContentUris.withAppendedId(Contacts.CONTENT_URI, id), null, null)
    }

    /**
     * Rewrites every [PERSON_MIME] row an older version wrote, whose `synced` held the email
     * addresses as `addresses`, with them as `emails`, so the first merge after the update does
     * not take every email as changed. Returns how many rows it rewrote; once they are rewritten
     * it finds none.
     */
    fun migrateSynced(): Int {
        val old = resolver.rows(
            Data.CONTENT_URI,
            arrayOf(Data._ID, Data.DATA2, Data.DATA3),
            "${Data.MIMETYPE} = ? AND (${Data.DATA3} IS NULL OR ${Data.DATA3} != ?)",
            arrayOf(PERSON_MIME, SYNCED_FORMAT),
        ) { c -> c.long(Data._ID) to upgraded(c.string(Data.DATA2)?.let(::parse), c.string(Data.DATA3)) }
        if (old.isEmpty()) return 0
        val ops = old.map { (id, synced) ->
            ContentProviderOperation.newUpdate(ContentUris.withAppendedId(Data.CONTENT_URI, id))
                .withValue(Data.DATA2, synced?.toString())
                .withValue(Data.DATA3, SYNCED_FORMAT)
                .build()
        }
        resolver.applyBatch(ContactsContract.AUTHORITY, ArrayList(ops))
        return old.size
    }

    /**
     * The one-time cleanup after joining vault [vaultId] (ticket 143): removes every
     * [PERSON_MIME] row of another vault, and every row of an older version, which names no
     * vault, whose person the local vault does not have; an older row whose person it has is
     * marked as [vaultId]'s. Returns how many rows it removed.
     */
    fun forgetOtherVaults(vaultId: String): Int {
        val rows = resolver.rows(
            Data.CONTENT_URI,
            arrayOf(Data._ID, Data.DATA1, VAULT),
            "${Data.MIMETYPE} = ?",
            arrayOf(PERSON_MIME),
        ) { c -> Triple(c.long(Data._ID), c.string(Data.DATA1), c.string(VAULT)) }
        val ops = ArrayList<ContentProviderOperation>()
        var removed = 0
        for ((id, person, vault) in rows) {
            val uri = ContentUris.withAppendedId(Data.CONTENT_URI, id)
            when {
                vault == vaultId -> {}
                vault == null && person != null && links.personExists(person) ->
                    ops.add(ContentProviderOperation.newUpdate(uri).withValue(VAULT, vaultId).build())
                else -> {
                    ops.add(ContentProviderOperation.newDelete(uri).build())
                    removed++
                }
            }
        }
        if (ops.isNotEmpty()) resolver.applyBatch(ContactsContract.AUTHORITY, ops)
        return removed
    }

    /**
     * Whether [row] links its contact to a person of the vault [vault] (the current one, or null
     * when not known yet): a row naming a vault belongs to that vault; one naming none, from an
     * older version, belongs where its person is.
     */
    private fun belongs(row: PersonLink, vault: String? = links.vaultId()): Boolean {
        val person = row.person ?: return false
        return if (row.vault != null && vault != null) row.vault == vault else links.personExists(person)
    }

    private fun find(contactId: String): Triple<Long, String?, Long> {
        val id = contactId.toLongOrNull() ?: throw IllegalArgumentException("$contactId is not a contact id")
        return resolver.rows(
            Contacts.CONTENT_URI,
            columns,
            "${Contacts._ID} = ?",
            arrayOf(id.toString()),
            limit = 1,
        ) { c -> Triple(id, c.string(Contacts.DISPLAY_NAME), c.long(Contacts.CONTACT_LAST_UPDATED_TIMESTAMP)) }
            .firstOrNull() ?: throw NoSuchElementException("no contact $contactId")
    }

    /** The raw contacts making up contact [contactId], oldest first. */
    private fun rawContactsOf(contactId: String): List<Long> = resolver.rows(
        RawContacts.CONTENT_URI,
        arrayOf(RawContacts._ID),
        "${RawContacts.CONTACT_ID} = ?",
        arrayOf(contactId),
        "${RawContacts._ID} ASC",
    ) { it.long(RawContacts._ID) }.ifEmpty { throw NoSuchElementException("no contact $contactId") }

    /** The contact holding person note [person] of the current vault, if any. */
    private fun contactOfPerson(person: String): String? {
        val vault = links.vaultId()
        return resolver.rows(
            Data.CONTENT_URI,
            arrayOf(Data.CONTACT_ID, VAULT),
            "${Data.MIMETYPE} = ? AND ${Data.DATA1} = ?",
            arrayOf(PERSON_MIME, person),
        ) { c -> c.long(Data.CONTACT_ID).toString() to c.string(VAULT) }
            .firstOrNull { (_, v) -> v == null || vault == null || v == vault }?.first
    }

    /** The contact's [PERSON_MIME] row, whichever vault it belongs to. */
    private fun personRow(contactId: Long): PersonLink? = resolver.rows(
        Data.CONTENT_URI,
        arrayOf(Data.DATA1, Data.DATA2, Data.DATA3, VAULT),
        "${Data.CONTACT_ID} = ? AND ${Data.MIMETYPE} = ?",
        arrayOf(contactId.toString(), PERSON_MIME),
        limit = 1,
    ) { c -> PersonLink(c.string(Data.DATA1), c.string(Data.DATA2)?.let(::parse), c.string(Data.DATA3), c.string(VAULT)) }.firstOrNull()

    private fun deleteRows(raws: List<Long>, mime: String, extra: Pair<String, Array<String>>? = null): ContentProviderOperation =
        ContentProviderOperation.newDelete(Data.CONTENT_URI)
            .withSelection(
                "${Data.RAW_CONTACT_ID} IN (${raws.joinToString(",") { "?" }}) AND ${Data.MIMETYPE} = ?" +
                    (extra?.let { " AND ${it.first}" } ?: ""),
                (raws.map(Long::toString) + mime + extra?.second.orEmpty()).toTypedArray(),
            )
            .build()

    /** A data row insert: into [rawId], or into the raw contact inserted first in the batch. */
    private fun row(rawId: Long?, mime: String): ContentProviderOperation.Builder {
        val b = ContentProviderOperation.newInsert(Data.CONTENT_URI).withValue(Data.MIMETYPE, mime)
        return if (rawId == null) b.withValueBackReference(Data.RAW_CONTACT_ID, 0) else b.withValue(Data.RAW_CONTACT_ID, rawId)
    }

    /** Writes [bytes] as raw contact [rawId]'s full-size photo; the provider makes the thumbnail. */
    private fun writePhoto(rawId: Long, bytes: ByteArray) {
        val uri = Uri.withAppendedPath(
            ContentUris.withAppendedId(RawContacts.CONTENT_URI, rawId),
            RawContacts.DisplayPhoto.CONTENT_DIRECTORY,
        )
        val fd = resolver.openAssetFileDescriptor(uri, "rw") ?: throw IllegalStateException("the phone took no photo for contact $rawId")
        fd.use { it.createOutputStream().use { out -> out.write(bytes) } }
    }

    /** The contact's full-size photo, or its thumbnail when it has no full-size one. */
    private fun photoBytes(id: Long): ByteArray? {
        val display = Uri.withAppendedPath(ContentUris.withAppendedId(Contacts.CONTENT_URI, id), Contacts.Photo.DISPLAY_PHOTO)
        val full = try {
            resolver.openInputStream(display)?.use { it.readBytes() }
        } catch (_: IOException) {
            null
        }
        if (full != null && full.isNotEmpty()) return full
        return resolver.rows(
            Data.CONTENT_URI,
            arrayOf(Photo.PHOTO),
            "${Data.CONTACT_ID} = ? AND ${Data.MIMETYPE} = ? AND ${Photo.PHOTO} IS NOT NULL",
            arrayOf(id.toString(), Photo.CONTENT_ITEM_TYPE),
            limit = 1,
        ) { c -> c.getColumnIndex(Photo.PHOTO).takeIf { it >= 0 && !c.isNull(it) }?.let { c.getBlob(it) } }
            .firstOrNull()?.takeIf { it.isNotEmpty() }
    }

    /** Every detail of contact [id] as the phone holds it, with the photo's bytes hashed but not stored. */
    private fun read(id: Long, name: String?, updated: Long): Details {
        val rows = resolver.rows(
            Data.CONTENT_URI,
            arrayOf(Data.MIMETYPE, Data.DATA1, Data.DATA2, Data.DATA3, Data.DATA4),
            "${Data.CONTACT_ID} = ?",
            arrayOf(id.toString()),
            "${Data._ID} ASC",
        ) { c -> DataRow(c.string(Data.MIMETYPE) ?: "", c.string(Data.DATA1), c.string(Data.DATA2), c.string(Data.DATA3), c.string(Data.DATA4)) }
        fun of(mime: String) = rows.filter { it.mime == mime }
        val lists = LIST_KINDS.associate { kind ->
            val seen = LinkedHashMap<String, String>()
            for (r in of(kind.mime)) {
                val value = kind.read(r.data1, numbers) ?: continue
                if (value !in seen) seen[value] = kind.labelOf(r.data2?.toIntOrNull(), r.data3)
            }
            kind.key to Labelled(seen.keys.toList(), seen.values.toList())
        }
        val org = of(Organization.CONTENT_ITEM_TYPE).firstOrNull { !it.data1.isNullOrBlank() || !it.data4.isNullOrBlank() }
        val birthday = of(Event.CONTENT_ITEM_TYPE)
            .filter { it.data2?.toIntOrNull() == Event.TYPE_BIRTHDAY }
            .firstNotNullOfOrNull { birthday(it.data1) } ?: ""
        val photo = photoBytes(id)?.let { Picture(it, ContactPhotos.sha256(it)) }
        return Details(
            name = name ?: "",
            lists = lists,
            organisation = org?.data1 ?: "",
            jobTitle = org?.data4 ?: "",
            nickname = of(Nickname.CONTENT_ITEM_TYPE).firstNotNullOfOrNull { it.data1?.takeIf(String::isNotBlank) } ?: "",
            birthday = birthday,
            websites = of(Website.CONTENT_ITEM_TYPE).mapNotNull { it.data1?.trim()?.takeIf(String::isNotEmpty) }.distinct(),
            note = of(Note.CONTENT_ITEM_TYPE).firstNotNullOfOrNull { it.data1?.takeIf(String::isNotEmpty) } ?: "",
            photo = photo,
            updated = updated,
        )
    }

    private fun contact(id: Long, name: String?, updated: Long): JSONObject = payload(id, read(id, name, updated), store = true)

    /** [details] as a payload; with [store], a photo not stored yet is stored as an asset. */
    private fun payload(id: Long, details: Details, store: Boolean): JSONObject {
        // A link to another vault's person reads as no link at all (ticket 143).
        val vault = personRow(id)?.takeIf { belongs(it) }
        val photo = details.photo?.let { if (store) photos.store(id, it) else photos.known(it) }
        val out = JSONObject()
            .put("contact_id", id.toString())
            .put("name", details.name)
        details.put(out, photo)
        return out
            .put("updated", Payloads.time(details.updated))
            .put("person", vault?.person ?: JSONObject.NULL)
            .put("synced", upgraded(vault?.synced, vault?.format) ?: JSONObject.NULL)
    }

    /** What an upsert asks for, each null when its key is left out, with labels settled. */
    private fun wanted(contact: JSONObject, now: Details): Wanted {
        fun str(key: String): String? = if (contact.has(key)) (if (contact.isNull(key)) "" else contact.optString(key)) else null
        val lists = LIST_KINDS.mapNotNull { kind ->
            if (!contact.has(kind.key)) return@mapNotNull null
            val given = contact.optJSONArray(kind.key).strings()
            val labels = if (contact.has(kind.labelKey)) contact.optJSONArray(kind.labelKey).allStrings() else null
            val current = now.lists.getValue(kind.key)
            val seen = LinkedHashMap<String, String>()
            given.forEachIndexed { i, raw ->
                val value = if (kind.key == "phones") raw else kind.read(raw, numbers) ?: return@forEachIndexed
                if (value in seen) return@forEachIndexed
                val label = labels?.getOrNull(i)?.trim()?.takeIf(String::isNotEmpty)
                    ?: current.values.indexOf(value).takeIf { it >= 0 }?.let { current.labels[it] }
                    ?: kind.defaultLabel
                seen[value] = label
            }
            kind.key to Labelled(seen.keys.toList(), seen.values.toList())
        }.toMap()
        return Wanted(
            name = if (contact.has("name")) contact.optString("name") else null,
            lists = lists,
            organisation = str("organisation"),
            jobTitle = str("job_title"),
            nickname = str("nickname"),
            birthday = str("birthday")?.let { birthday(it) ?: "" },
            websites = if (contact.has("websites")) contact.optJSONArray("websites").strings().map(String::trim).distinct() else null,
            note = str("contact_note"),
            photo = contact.has("photo"),
        )
    }

    /** The contact's detail values, as [read] gives them. */
    private data class Details(
        val name: String,
        val lists: Map<String, Labelled>,
        val organisation: String,
        val jobTitle: String,
        val nickname: String,
        val birthday: String,
        val websites: List<String>,
        val note: String,
        val photo: Picture?,
        val updated: Long,
    ) {
        val phones: Labelled get() = lists.getValue("phones")

        /** The detail keys, as a payload has them, with [photo] as `{ asset, hash }`. */
        fun put(out: JSONObject, photo: JSONObject?) {
            for (kind in LIST_KINDS) {
                val l = lists.getValue(kind.key)
                out.put(kind.key, Payloads.list(l.values)).put(kind.labelKey, Payloads.list(l.labels))
            }
            out.put("organisation", organisation)
                .put("job_title", jobTitle)
                .put("nickname", nickname)
                .put("birthday", birthday)
                .put("websites", Payloads.list(websites))
                .put("contact_note", note)
                .put("photo", photo ?: JSONObject.NULL)
        }

        companion object {
            val EMPTY = Details("", LIST_KINDS.associate { it.key to Labelled(emptyList(), emptyList()) }, "", "", "", "", emptyList(), "", null, 0)
        }
    }

    private data class Wanted(
        val name: String?,
        val lists: Map<String, Labelled>,
        val organisation: String?,
        val jobTitle: String?,
        val nickname: String?,
        val birthday: String?,
        val websites: List<String>?,
        val note: String?,
        val photo: Boolean,
    ) {
        /** The `synced` members for the keys asked for: what the contact now holds of each. */
        fun agreed(now: Details, photo: JSONObject?): List<Pair<String, Any>> = buildList {
            name?.let { add("name" to it) }
            for (kind in LIST_KINDS) {
                lists[kind.key]?.let {
                    add(kind.key to Payloads.list(it.values))
                    add(kind.labelKey to Payloads.list(it.labels))
                }
            }
            if (organisation != null || jobTitle != null) {
                add("organisation" to (organisation ?: now.organisation))
                add("job_title" to (jobTitle ?: now.jobTitle))
            }
            nickname?.let { add("nickname" to it) }
            birthday?.let { add("birthday" to it) }
            websites?.let { add("websites" to Payloads.list(it)) }
            note?.let { add("contact_note" to it) }
            if (this@Wanted.photo) add("photo" to (photo ?: JSONObject.NULL))
        }
    }

    private data class Labelled(val values: List<String>, val labels: List<String>)

    /** A [PERSON_MIME] row: the person's id, what the sides last agreed on, its format, and its vault. */
    private data class PersonLink(val person: String?, val synced: JSONObject?, val format: String?, val vault: String?)

    private data class DataRow(val mime: String, val data1: String?, val data2: String?, val data3: String?, val data4: String?)

    /** A contact photo's bytes and their SHA-256. */
    data class Picture(val bytes: ByteArray, val hash: String) {
        override fun equals(other: Any?) = other is Picture && other.hash == hash
        override fun hashCode() = hash.hashCode()
    }

    /** A kind of detail held as a labelled list: phones, emails, postal addresses. */
    private class ListKind(
        val key: String,
        val labelKey: String,
        val mime: String,
        val valueColumn: String,
        /** The label of each type this kind has; any other type is "other". */
        val types: Map<Int, String>,
        val defaultLabel: String,
        val read: (String?, PhoneNumbers) -> String?,
    ) {
        fun labelOf(type: Int?, label: String?): String = when (type) {
            BaseTypes.TYPE_CUSTOM -> label?.trim()?.takeIf(String::isNotEmpty) ?: OTHER
            null -> defaultLabel
            else -> types[type] ?: OTHER
        }

        /** The type a label is written as: its own, or custom with the label as its text. */
        fun typeOf(label: String): Int = types.entries.firstOrNull { it.value == label }?.key ?: BaseTypes.TYPE_CUSTOM
    }

    companion object {
        private const val TAG = "VaultContacts"

        /** The MIME type of the data row that links a contact to its `person` note. */
        const val PERSON_MIME = "vnd.android.cursor.item/vnd.dev.wiltaylor.thevault.person"

        /** The `DATA3` of a [PERSON_MIME] row whose `synced` names email addresses `emails`. */
        const val SYNCED_FORMAT = "2"

        /** The column of a [PERSON_MIME] row holding the id of the vault its person is in. */
        const val VAULT = Data.DATA4

        private const val OTHER = "other"
        private const val TYPE = Data.DATA2
        private const val LABEL = Data.DATA3

        private val LIST_KINDS = listOf(
            ListKind(
                "phones", "phone_labels", Phone.CONTENT_ITEM_TYPE, Phone.NUMBER,
                mapOf(Phone.TYPE_MOBILE to "mobile", Phone.TYPE_HOME to "home", Phone.TYPE_WORK to "work", Phone.TYPE_MAIN to "main", Phone.TYPE_OTHER to OTHER),
                "mobile",
            ) { raw, numbers -> numbers.normalise(raw) },
            ListKind(
                "emails", "email_labels", Email.CONTENT_ITEM_TYPE, Email.ADDRESS,
                mapOf(Email.TYPE_MOBILE to "mobile", Email.TYPE_HOME to "home", Email.TYPE_WORK to "work", Email.TYPE_OTHER to OTHER),
                OTHER,
            ) { raw, _ -> raw?.trim()?.takeIf(String::isNotEmpty) },
            ListKind(
                "addresses", "address_labels", StructuredPostal.CONTENT_ITEM_TYPE, StructuredPostal.FORMATTED_ADDRESS,
                mapOf(StructuredPostal.TYPE_HOME to "home", StructuredPostal.TYPE_WORK to "work", StructuredPostal.TYPE_OTHER to OTHER),
                "home",
            ) { raw, _ -> postal(raw) },
        )

        /** A formatted postal address on one line: its lines joined by ", ". */
        fun postal(raw: String?): String? =
            raw?.split('\n', '\r')?.map(String::trim)?.filter(String::isNotEmpty)?.joinToString(", ")?.takeIf(String::isNotEmpty)

        private val FULL_DATE = Regex("""^(\d{4})-?(\d{2})-?(\d{2})(?:$|[T ].*)""")
        private val NO_YEAR = Regex("""^--(\d{2})-?(\d{2})$""")

        /** A birthday as "YYYY-MM-DD", or "--MM-DD" with no year; null when [raw] is neither. */
        fun birthday(raw: String?): String? {
            val s = raw?.trim().orEmpty()
            if (s.isEmpty()) return null
            FULL_DATE.matchEntire(s)?.let { val (y, m, d) = it.destructured; return "$y-$m-$d" }
            NO_YEAR.matchEntire(s)?.let { val (m, d) = it.destructured; return "--$m-$d" }
            return null
        }

        private fun parse(text: String): JSONObject? = runCatching { JSONObject(text) }.getOrNull()

        /** A `synced` as this version writes it: an older one's `addresses` are its `emails`. */
        private fun upgraded(synced: JSONObject?, format: String?): JSONObject? {
            if (synced == null || format == SYNCED_FORMAT) return synced?.let { JSONObject(it.toString()) }
            val out = JSONObject(synced.toString())
            if (!out.has("emails") && out.has("addresses")) {
                out.put("emails", out.get("addresses"))
                out.remove("addresses")
            }
            return out
        }
    }
}

/**
 * Which vault a contact's person link must belong to for [ContactsReader] to report it (ticket
 * 143). The defaults take every link as the current vault's, as before vaults were named.
 */
class ContactLinks(
    /** The id of the home server's vault the local vault is a copy of, or null before it is known. */
    val vaultId: () -> String? = { null },
    /** Whether the local vault holds person note [id]: decides a link row that names no vault. */
    val personExists: (String) -> Boolean = { true },
)

/**
 * Contact photos as assets of the local vault: each photo's bytes are stored once, and a photo
 * read again with the same SHA-256 reuses the asset stored for it, remembered in [known] by
 * hash. A photo written to a contact from an asset is remembered under that asset too.
 */
class ContactPhotos(private val assets: AssetStore, private val known: MutableMap<String, String> = HashMap()) {
    /** `{ asset, hash }` for [picture], storing its bytes unless an asset already holds them. */
    fun store(contactId: Long, picture: ContactsReader.Picture): JSONObject {
        val asset = synchronized(known) { known[picture.hash] } ?: assets
            .put(FileNames.withExtension("contact-$contactId-photo", mimeOf(picture.bytes)), picture.bytes)
            .getString("asset")
            .also { remember(picture.hash, it) }
        return JSONObject().put("asset", asset).put("hash", picture.hash)
    }

    /** `{ asset, hash }` for [picture] without storing it: `asset` is null when none holds it. */
    fun known(picture: ContactsReader.Picture): JSONObject =
        JSONObject().put("asset", assetOf(picture.hash) ?: JSONObject.NULL).put("hash", picture.hash)

    fun assetOf(hash: String): String? = synchronized(known) { known[hash] }

    fun remember(hash: String, asset: String) {
        synchronized(known) { known[hash] = asset }
    }

    /** The bytes of [asset], or null when the vault does not hold it. */
    fun bytes(asset: String): ByteArray? = assets.bytes(asset)

    companion object {
        fun sha256(bytes: ByteArray): String =
            MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

        /** The image type of [bytes] from their first bytes, JPEG when unknown. */
        fun mimeOf(bytes: ByteArray): String = when {
            bytes.size >= 4 && bytes[0] == 0x89.toByte() && bytes[1] == 'P'.code.toByte() -> "image/png"
            bytes.size >= 12 && String(bytes, 8, 4, Charsets.US_ASCII) == "WEBP" -> "image/webp"
            bytes.size >= 3 && String(bytes, 0, 3, Charsets.US_ASCII) == "GIF" -> "image/gif"
            else -> "image/jpeg"
        }
    }
}

/** A [MutableMap] of strings kept in [prefs], under keys starting [prefix]. */
class PrefsMap(private val prefs: SharedPreferences, private val prefix: String) : AbstractMutableMap<String, String>() {
    override val entries: MutableSet<MutableMap.MutableEntry<String, String>>
        get() = prefs.all.filterKeys { it.startsWith(prefix) }
            .mapNotNull { (k, v) -> (v as? String)?.let { java.util.AbstractMap.SimpleEntry(k.removePrefix(prefix), it) } }
            .toMutableSet()

    override fun get(key: String): String? = prefs.getString(prefix + key, null)

    override fun put(key: String, value: String): String? {
        val before = get(key)
        prefs.edit().putString(prefix + key, value).apply()
        return before
    }
}

private fun JSONArray?.strings(): List<String> =
    if (this == null) emptyList() else (0 until length()).mapNotNull { optString(it).takeIf(String::isNotEmpty) }

/** Every member as a string, keeping empty ones, so a label list stays aligned with its values. */
private fun JSONArray?.allStrings(): List<String> =
    if (this == null) emptyList() else (0 until length()).map { if (isNull(it)) "" else optString(it) }
