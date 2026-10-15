package dev.wiltaylor.thevault.device

import android.content.ContentProviderOperation
import android.content.ContentResolver
import android.content.ContentUris
import android.net.Uri
import android.provider.ContactsContract
import android.provider.ContactsContract.CommonDataKinds.Email
import android.provider.ContactsContract.CommonDataKinds.Phone
import android.provider.ContactsContract.CommonDataKinds.StructuredName
import android.provider.ContactsContract.Contacts
import android.provider.ContactsContract.Data
import android.provider.ContactsContract.DeletedContacts
import android.provider.ContactsContract.PhoneLookup
import android.provider.ContactsContract.RawContacts
import org.json.JSONArray
import org.json.JSONObject

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
 * `contact_changed` and `contact_deleted` payloads: `{ contact_id, name, phones, addresses,
 * updated, person, synced }`, with `phones` in E.164 and `addresses` the email addresses.
 *
 * A contact linked to a `person` note carries one data row of its own, [PERSON_MIME], holding
 * the note's id and what the contact and the person last agreed on (D16, D24): `person` and
 * `synced` in the payload, both null on a contact never linked. The contacts app does not show
 * the row.
 *
 * A changed contact's key is its last-updated time, and a deleted one's its deletion time, in
 * milliseconds: the provider gives no other order for changes.
 */
class ContactsReader(private val resolver: ContentResolver, private val numbers: PhoneNumbers) {
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
        ) { c ->
            val updated = c.long(Contacts.CONTACT_LAST_UPDATED_TIMESTAMP)
            Captured("contact_changed", contact(c.long(Contacts._ID), c.string(Contacts.DISPLAY_NAME), updated), updated, updated)
        }

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
        ) { c ->
            contact(c.long(Contacts._ID), c.string(Contacts.DISPLAY_NAME), c.long(Contacts.CONTACT_LAST_UPDATED_TIMESTAMP))
        }
        return all.filter { number == null || it.getJSONArray("phones").contains(number) }
            .take(filter.limit())
    }

    /** `device::contacts_get`. */
    fun get(contactId: String): JSONObject {
        val id = contactId.toLongOrNull() ?: throw IllegalArgumentException("$contactId is not a contact id")
        return resolver.rows(
            Contacts.CONTENT_URI,
            columns,
            "${Contacts._ID} = ?",
            arrayOf(id.toString()),
            limit = 1,
        ) { c ->
            contact(id, c.string(Contacts.DISPLAY_NAME), c.long(Contacts.CONTACT_LAST_UPDATED_TIMESTAMP))
        }.firstOrNull() ?: throw NoSuchElementException("no contact $contactId")
    }

    /**
     * `device::contacts_upsert`: add the contact, or change contact `contact_id`, to hold the
     * name, phones and addresses given. Only the kinds that differ are rewritten, so a contact
     * that already agrees keeps its own number formatting and labels. With `person`, the
     * contact's [PERSON_MIME] row is set to the note's id and to these fields as what the two
     * sides now agree on, and a contact with no `contact_id` that already holds that person is
     * changed rather than added twice. Returns `{ contact_id }`.
     */
    fun upsert(contact: JSONObject): JSONObject {
        val name = contact.optString("name")
        val phones = contact.optJSONArray("phones").strings()
        val addresses = contact.optJSONArray("addresses").strings()
        val person = contact.text("person")
        val synced = JSONObject()
            .put("name", name)
            .put("phones", Payloads.list(phones))
            .put("addresses", Payloads.list(addresses))
        val ops = ArrayList<ContentProviderOperation>()
        val existing = contact.text("contact_id") ?: person?.let(::contactOfPerson)
        if (existing == null) {
            ops.add(
                ContentProviderOperation.newInsert(RawContacts.CONTENT_URI)
                    .withValue(RawContacts.ACCOUNT_TYPE, null)
                    .withValue(RawContacts.ACCOUNT_NAME, null)
                    .build(),
            )
            addName(ops, null, name)
            addPhones(ops, null, phones)
            addAddresses(ops, null, addresses)
            if (person != null) addPerson(ops, null, person, synced)
            val results = resolver.applyBatch(ContactsContract.AUTHORITY, ops)
            val rawId = results.firstOrNull()?.uri?.let { ContentUris.parseId(it) }
                ?: throw IllegalStateException("the phone did not add the contact")
            val contactId = resolver.rows(
                RawContacts.CONTENT_URI,
                arrayOf(RawContacts.CONTACT_ID),
                "${RawContacts._ID} = ?",
                arrayOf(rawId.toString()),
                limit = 1,
            ) { it.long(RawContacts.CONTACT_ID) }.firstOrNull()
                ?: throw IllegalStateException("the phone added the contact but has no id for it")
            return JSONObject().put("contact_id", contactId.toString())
        }
        val now = get(existing)
        val raws = rawContactsOf(existing)
        val rawId = raws.first()
        if (now.optString("name") != name) {
            ops.add(deleteRows(raws, StructuredName.CONTENT_ITEM_TYPE))
            addName(ops, rawId, name)
        }
        if (now.getJSONArray("phones").strings() != phones) {
            ops.add(deleteRows(raws, Phone.CONTENT_ITEM_TYPE))
            addPhones(ops, rawId, phones)
        }
        if (now.getJSONArray("addresses").strings() != addresses) {
            ops.add(deleteRows(raws, Email.CONTENT_ITEM_TYPE))
            addAddresses(ops, rawId, addresses)
        }
        if (person != null && (now.optString("person") != person || now.optJSONObject("synced")?.toString() != synced.toString())) {
            ops.add(deleteRows(raws, PERSON_MIME))
            addPerson(ops, rawId, person, synced)
        }
        if (ops.isNotEmpty()) resolver.applyBatch(ContactsContract.AUTHORITY, ops)
        return JSONObject().put("contact_id", existing)
    }

    /** `device::contacts_delete`. */
    fun delete(contactId: String) {
        val id = contactId.toLongOrNull() ?: throw IllegalArgumentException("$contactId is not a contact id")
        resolver.delete(ContentUris.withAppendedId(Contacts.CONTENT_URI, id), null, null)
    }

    /** The raw contacts making up contact [contactId], oldest first. */
    private fun rawContactsOf(contactId: String): List<Long> = resolver.rows(
        RawContacts.CONTENT_URI,
        arrayOf(RawContacts._ID),
        "${RawContacts.CONTACT_ID} = ?",
        arrayOf(contactId),
        "${RawContacts._ID} ASC",
    ) { it.long(RawContacts._ID) }.ifEmpty { throw NoSuchElementException("no contact $contactId") }

    /** The contact holding person note [person], if any. */
    private fun contactOfPerson(person: String): String? = resolver.rows(
        Data.CONTENT_URI,
        arrayOf(Data.CONTACT_ID),
        "${Data.MIMETYPE} = ? AND ${Data.DATA1} = ?",
        arrayOf(PERSON_MIME, person),
        limit = 1,
    ) { it.long(Data.CONTACT_ID).toString() }.firstOrNull()

    /** The contact's [PERSON_MIME] row: the person's id and what the sides last agreed on. */
    private fun personRow(contactId: Long): Pair<String?, JSONObject?>? = resolver.rows(
        Data.CONTENT_URI,
        arrayOf(Data.DATA1, Data.DATA2),
        "${Data.CONTACT_ID} = ? AND ${Data.MIMETYPE} = ?",
        arrayOf(contactId.toString(), PERSON_MIME),
        limit = 1,
    ) { c ->
        val synced = c.string(Data.DATA2)?.let { runCatching { JSONObject(it) }.getOrNull() }
        c.string(Data.DATA1) to synced
    }.firstOrNull()

    private fun deleteRows(raws: List<Long>, mime: String): ContentProviderOperation =
        ContentProviderOperation.newDelete(Data.CONTENT_URI)
            .withSelection(
                "${Data.RAW_CONTACT_ID} IN (${raws.joinToString(",") { "?" }}) AND ${Data.MIMETYPE} = ?",
                (raws.map(Long::toString) + mime).toTypedArray(),
            )
            .build()

    /** A data row insert: into [rawId], or into the raw contact inserted first in the batch. */
    private fun row(rawId: Long?, mime: String): ContentProviderOperation.Builder {
        val b = ContentProviderOperation.newInsert(Data.CONTENT_URI).withValue(Data.MIMETYPE, mime)
        return if (rawId == null) b.withValueBackReference(Data.RAW_CONTACT_ID, 0) else b.withValue(Data.RAW_CONTACT_ID, rawId)
    }

    private fun addName(ops: ArrayList<ContentProviderOperation>, rawId: Long?, name: String) {
        ops.add(row(rawId, StructuredName.CONTENT_ITEM_TYPE).withValue(StructuredName.DISPLAY_NAME, name).build())
    }

    private fun addPhones(ops: ArrayList<ContentProviderOperation>, rawId: Long?, phones: List<String>) = phones.forEach {
        ops.add(row(rawId, Phone.CONTENT_ITEM_TYPE).withValue(Phone.NUMBER, it).withValue(Phone.TYPE, Phone.TYPE_MOBILE).build())
    }

    private fun addAddresses(ops: ArrayList<ContentProviderOperation>, rawId: Long?, addresses: List<String>) = addresses.forEach {
        ops.add(row(rawId, Email.CONTENT_ITEM_TYPE).withValue(Email.ADDRESS, it).withValue(Email.TYPE, Email.TYPE_OTHER).build())
    }

    private fun addPerson(ops: ArrayList<ContentProviderOperation>, rawId: Long?, person: String, synced: JSONObject) {
        ops.add(row(rawId, PERSON_MIME).withValue(Data.DATA1, person).withValue(Data.DATA2, synced.toString()).build())
    }

    private fun contact(id: Long, name: String?, updated: Long): JSONObject {
        val args = arrayOf(id.toString())
        val phones = resolver.rows(Phone.CONTENT_URI, arrayOf(Phone.NUMBER), "${Phone.CONTACT_ID} = ?", args) {
            numbers.normalise(it.string(Phone.NUMBER))
        }.distinct()
        val addresses = resolver.rows(Email.CONTENT_URI, arrayOf(Email.ADDRESS), "${Email.CONTACT_ID} = ?", args) {
            it.string(Email.ADDRESS)
        }.distinct()
        val vault = personRow(id)
        return JSONObject()
            .put("contact_id", id.toString())
            .put("name", name ?: "")
            .put("phones", Payloads.list(phones))
            .put("addresses", Payloads.list(addresses))
            .put("updated", Payloads.time(updated))
            .put("person", vault?.first ?: JSONObject.NULL)
            .put("synced", vault?.second ?: JSONObject.NULL)
    }

    companion object {
        /** The MIME type of the data row that links a contact to its `person` note. */
        const val PERSON_MIME = "vnd.android.cursor.item/vnd.dev.wiltaylor.thevault.person"
    }
}

private fun JSONArray?.strings(): List<String> =
    if (this == null) emptyList() else (0 until length()).mapNotNull { optString(it).takeIf(String::isNotEmpty) }

private fun JSONArray.contains(value: String): Boolean = (0 until length()).any { optString(it) == value }
