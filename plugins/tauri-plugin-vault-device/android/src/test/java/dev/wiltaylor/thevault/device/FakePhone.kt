package dev.wiltaylor.thevault.device

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.sqlite.SQLiteDatabase
import android.net.Uri
import android.os.ParcelFileDescriptor
import org.json.JSONObject
import org.robolectric.Robolectric
import org.robolectric.Shadows.shadowOf
import java.io.ByteArrayInputStream
import java.io.File
import java.util.concurrent.Executor

/**
 * Fake phone providers for S5: an in-memory SQLite database behind the real authorities and
 * column names of the Telephony provider (SMS, MMS and its parts and addresses), the call log and
 * contacts, so readers run their real selections and orders against it.
 */
class FakePhone(private val context: Context) {
    val db: SQLiteDatabase = SQLiteDatabase.create(null)
    val sink = RecordingSink()
    val assets = RecordingAssets()

    init {
        db.execSQL("CREATE TABLE sms (_id INTEGER PRIMARY KEY, thread_id INTEGER, address TEXT, body TEXT, date INTEGER, date_sent INTEGER, type INTEGER)")
        db.execSQL("CREATE TABLE mms (_id INTEGER PRIMARY KEY, thread_id INTEGER, date INTEGER, date_sent INTEGER, sub TEXT, msg_box INTEGER)")
        db.execSQL("CREATE TABLE addr (_id INTEGER PRIMARY KEY, msg_id INTEGER, address TEXT, type INTEGER)")
        db.execSQL("CREATE TABLE part (_id INTEGER PRIMARY KEY, mid INTEGER, ct TEXT, name TEXT, fn TEXT, cl TEXT, text TEXT)")
        db.execSQL("CREATE TABLE calls (_id INTEGER PRIMARY KEY, number TEXT, type INTEGER, date INTEGER, duration INTEGER)")
        db.execSQL("CREATE TABLE contacts (_id INTEGER PRIMARY KEY, display_name TEXT, contact_last_updated_timestamp INTEGER)")
        // Like Android, each contact is raw contacts holding typed data rows. Here a raw contact
        // the tests add has its contact's id, and one the app adds becomes a contact of its own.
        db.execSQL("CREATE TABLE raw_contacts (_id INTEGER PRIMARY KEY, contact_id INTEGER, account_type TEXT, account_name TEXT)")
        db.execSQL("CREATE TABLE data (_id INTEGER PRIMARY KEY, raw_contact_id INTEGER, mimetype TEXT, data1 TEXT, data2 TEXT, data3 TEXT, data4 TEXT)")
        db.execSQL(
            "CREATE VIEW data_view AS SELECT data.*, raw_contacts.contact_id AS contact_id " +
                "FROM data JOIN raw_contacts ON raw_contacts._id = data.raw_contact_id",
        )
        db.execSQL("CREATE VIEW phones AS SELECT * FROM data_view WHERE mimetype = '$PHONE'")
        db.execSQL("CREATE VIEW emails AS SELECT * FROM data_view WHERE mimetype = '$EMAIL'")
        db.execSQL("CREATE TABLE deleted_contacts (contact_id INTEGER, contact_deleted_timestamp INTEGER)")
        db.execSQL(
            "CREATE VIEW phone_lookup AS SELECT contacts._id AS _id, display_name, data1 AS number, data4 AS normalized_number " +
                "FROM phones JOIN contacts ON contacts._id = phones.contact_id",
        )
        Provider.phone = this
        for (authority in listOf("sms", "mms", "mms-sms", "call_log", "com.android.contacts")) {
            Robolectric.buildContentProvider(Provider::class.java).create(authority)
        }
    }

    fun sms(id: Long, address: String, body: String, type: Int, date: Long = id * 60_000, thread: Long = 7) {
        db.execSQL("INSERT INTO sms VALUES (?, ?, ?, ?, ?, ?, ?)", arrayOf(id, thread, address, body, date, date - 5_000, type))
        notify(android.provider.Telephony.Sms.CONTENT_URI)
    }

    fun mms(id: Long, from: String, to: List<String>, dateSeconds: Long = id * 60, subject: String? = null, thread: Long = 9) {
        db.execSQL("INSERT INTO mms VALUES (?, ?, ?, ?, ?, ?)", arrayOf(id, thread, dateSeconds, dateSeconds, subject, 1))
        db.execSQL("INSERT INTO addr (msg_id, address, type) VALUES (?, ?, 137)", arrayOf(id, from))
        to.forEach { db.execSQL("INSERT INTO addr (msg_id, address, type) VALUES (?, ?, 151)", arrayOf(id, it)) }
    }

    fun part(id: Long, mms: Long, mime: String, text: String? = null, name: String? = null, bytes: ByteArray? = null) {
        db.execSQL("INSERT INTO part (_id, mid, ct, name, text) VALUES (?, ?, ?, ?, ?)", arrayOf(id, mms, mime, name, text))
        if (bytes != null) {
            Provider.files[id] = bytes
            val uri = Uri.parse("content://mms/part/$id")
            shadowOf(context.contentResolver).registerInputStreamSupplier(uri) { ByteArrayInputStream(bytes) }
        }
    }

    fun call(id: Long, number: String, type: Int, duration: Long, date: Long = id * 60_000) {
        db.execSQL("INSERT INTO calls VALUES (?, ?, ?, ?, ?)", arrayOf(id, number, type, date, duration))
    }

    /** Adds or replaces contact [id], as the contacts app does: its name, phones (raw to E.164) and emails. */
    fun contact(id: Long, name: String, updated: Long, phones: List<Pair<String, String>> = emptyList(), emails: List<String> = emptyList()) {
        db.execSQL("INSERT OR REPLACE INTO contacts VALUES (?, ?, ?)", arrayOf(id, name, updated))
        db.execSQL("INSERT OR IGNORE INTO raw_contacts (_id, contact_id) VALUES (?, ?)", arrayOf(id, id))
        db.execSQL("DELETE FROM data WHERE raw_contact_id = ? AND mimetype IN (?, ?, ?)", arrayOf(id, NAME, PHONE, EMAIL))
        db.execSQL("INSERT INTO data (raw_contact_id, mimetype, data1) VALUES (?, ?, ?)", arrayOf(id, NAME, name))
        phones.forEach { (raw, normalised) -> db.execSQL("INSERT INTO data (raw_contact_id, mimetype, data1, data4) VALUES (?, ?, ?, ?)", arrayOf(id, PHONE, raw, normalised)) }
        emails.forEach { db.execSQL("INSERT INTO data (raw_contact_id, mimetype, data1) VALUES (?, ?, ?)", arrayOf(id, EMAIL, it)) }
    }

    fun deleteContact(id: Long, at: Long) {
        db.execSQL("DELETE FROM data WHERE raw_contact_id IN (SELECT _id FROM raw_contacts WHERE contact_id = ?)", arrayOf(id))
        db.execSQL("DELETE FROM raw_contacts WHERE contact_id = ?", arrayOf(id))
        db.execSQL("DELETE FROM contacts WHERE _id = ?", arrayOf(id))
        db.execSQL("INSERT INTO deleted_contacts VALUES (?, ?)", arrayOf(id, at))
    }

    /** The data rows of contact [id] with [mime], as `data1 data2` text. */
    fun rows(id: Long, mime: String): List<String> =
        db.rawQuery("SELECT data1, data2 FROM data_view WHERE contact_id = ? AND mimetype = ? ORDER BY _id", arrayOf(id.toString(), mime)).use { c ->
            buildList { while (c.moveToNext()) add(listOfNotNull(c.getString(0), c.getString(1)).joinToString(" ")) }
        }

    /** Contact [id]'s display name, or null when there is no such contact. */
    fun name(id: Long): String? =
        db.rawQuery("SELECT display_name FROM contacts WHERE _id = ?", arrayOf(id.toString())).use { c ->
            if (c.moveToNext()) c.getString(0) else null
        }

    /** Every contact id, in order. */
    fun contactIds(): List<Long> = db.rawQuery("SELECT _id FROM contacts ORDER BY _id", null).use { c ->
        buildList { while (c.moveToNext()) add(c.getLong(0)) }
    }

    /** The contacts store's own bookkeeping after an app write: name and last-updated time. */
    private var clock = 1_000_000L

    internal fun touched(rawIds: Collection<Long>) {
        for (raw in rawIds) {
            val contact = db.rawQuery("SELECT contact_id FROM raw_contacts WHERE _id = ?", arrayOf(raw.toString())).use { c ->
                if (c.moveToNext()) c.getLong(0) else null
            } ?: continue
            val name = db.rawQuery(
                "SELECT data1 FROM data_view WHERE contact_id = ? AND mimetype = ? ORDER BY _id LIMIT 1",
                arrayOf(contact.toString(), NAME),
            ).use { c -> if (c.moveToNext()) c.getString(0) else null }
            db.execSQL("INSERT OR REPLACE INTO contacts VALUES (?, ?, ?)", arrayOf(contact, name, ++clock))
        }
    }

    fun notify(uri: Uri) = context.contentResolver.notifyChange(uri, null)

    /**
     * A capture over these providers into [into], as Australian numbers, with this phone's number
     * `+61400000000`.
     */
    fun capture(prefs: String = "cursors", into: EventSink = sink): DeviceCapture = DeviceCapture(
        context.contentResolver,
        PhoneNumbers("AU"),
        into,
        Cursors(context.getSharedPreferences(prefs, Context.MODE_PRIVATE)),
        assets,
        { setOf(SELF) },
    )

    /** An RCS capture over these providers, into [sink], remembering what it saw in [prefs]. */
    fun rcs(prefs: String = "rcs"): RcsCapture =
        RcsCapture(context.contentResolver, sink, SeenRefs(context.getSharedPreferences(prefs, Context.MODE_PRIVATE)))

    /** Route the process's capture through this phone into [into], on the calling thread. */
    fun install(into: EventSink = sink): DeviceCapture {
        val capture = capture(into = into)
        Device.reset()
        Device.factory = { capture }
        Device.rcsFactory = { rcs() }
        Device.executor = Executor { it.run() }
        Device.retryDelayMs = 0
        return capture
    }

    /** The routes of the real providers' URIs onto the tables above. */
    class Provider : ContentProvider() {
        override fun onCreate() = true

        override fun query(uri: Uri, projection: Array<String>?, selection: String?, args: Array<String>?, sort: String?): Cursor {
            val p = uri.pathSegments
            val (table, extra, extraArgs) = when (uri.authority) {
                "sms" -> Route("sms")
                "mms" -> when {
                    p.isEmpty() -> Route("mms")
                    p == listOf("part") -> Route("part")
                    p.size == 2 && p[1] == "addr" -> Route("addr", "msg_id = ?", listOf(p[0]))
                    else -> error("no fake for $uri")
                }
                "call_log" -> Route("calls")
                "com.android.contacts" -> when {
                    p == listOf("contacts") -> Route("contacts")
                    p == listOf("raw_contacts") -> Route("raw_contacts")
                    p == listOf("data") -> Route("data_view")
                    p == listOf("data", "phones") -> Route("phones")
                    p == listOf("data", "emails") -> Route("emails")
                    p == listOf("deleted_contacts") -> Route("deleted_contacts")
                    p.size == 2 && p[0] == "phone_lookup" -> Route(
                        // Like Android's lookup, ignoring how the number is spaced.
                        "phone_lookup",
                        "(replace(replace(number, ' ', ''), '-', '') = replace(replace(?, ' ', ''), '-', '') OR normalized_number = ?)",
                        listOf(p[1], p[1]),
                    )
                    else -> error("no fake for $uri")
                }
                else -> error("no fake for $uri")
            }
            val where = listOfNotNull(selection?.let { "($it)" }, extra).joinToString(" AND ").ifEmpty { null }
            return phone.db.query(table, projection, where, (args.orEmpty().toList() + extraArgs).toTypedArray(), null, null, sort)
        }

        override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
            val bytes = files[uri.lastPathSegment!!.toLong()] ?: error("no file for $uri")
            val file = File.createTempFile("part", null).apply { writeBytes(bytes); deleteOnExit() }
            return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
        }

        override fun getType(uri: Uri): String? = null

        /** Contacts writes: a raw contact, which becomes a contact of its own, or a data row. */
        override fun insert(uri: Uri, values: ContentValues?): Uri? {
            val v = values ?: ContentValues()
            return when (uri.authority to uri.pathSegments) {
                "com.android.contacts" to listOf("raw_contacts") -> {
                    val id = phone.db.insertOrThrow("raw_contacts", null, ContentValues(v).apply { remove("contact_id") })
                    phone.db.execSQL("UPDATE raw_contacts SET contact_id = _id WHERE _id = ?", arrayOf(id))
                    phone.touched(listOf(id))
                    Uri.withAppendedPath(uri, id.toString())
                }
                "com.android.contacts" to listOf("data") -> {
                    val id = phone.db.insertOrThrow("data", null, v)
                    phone.touched(listOf(v.getAsLong("raw_contact_id")))
                    Uri.withAppendedPath(uri, id.toString())
                }
                else -> error("no fake insert for $uri")
            }
        }

        override fun delete(uri: Uri, selection: String?, args: Array<String>?): Int {
            val p = uri.pathSegments
            return when {
                uri.authority == "com.android.contacts" && p == listOf("data") -> {
                    val raws = phone.db.query("data", arrayOf("raw_contact_id"), selection, args, null, null, null).use { c ->
                        buildSet { while (c.moveToNext()) add(c.getLong(0)) }
                    }
                    phone.db.delete("data", selection, args).also { phone.touched(raws) }
                }
                uri.authority == "com.android.contacts" && p.size == 2 && p[0] == "contacts" -> {
                    phone.deleteContact(p[1].toLong(), System.currentTimeMillis())
                    1
                }
                else -> error("no fake delete for $uri")
            }
        }

        override fun update(uri: Uri, values: ContentValues?, selection: String?, args: Array<String>?) = 0

        private data class Route(val table: String, val where: String? = null, val args: List<String> = emptyList())

        companion object {
            lateinit var phone: FakePhone
            val files = HashMap<Long, ByteArray>()
        }
    }

    companion object {
        const val SELF = "+61400000000"
        const val NAME = "vnd.android.cursor.item/name"
        const val PHONE = "vnd.android.cursor.item/phone_v2"
        const val EMAIL = "vnd.android.cursor.item/email_v2"
    }
}

/** The device event queue, as a list. */
class RecordingSink : EventSink {
    data class Event(val event: String, val data: JSONObject)

    val events = mutableListOf<Event>()
    var flushes = 0

    override fun enqueue(event: String, data: JSONObject, atMs: Long) {
        events.add(Event(event, data))
    }

    override fun flush() {
        flushes++
    }

    fun refs(): List<String> = events.map { it.data.optString("device_ref", it.data.optString("contact_id")) }
}

/** The vault's assets, as a list. */
class RecordingAssets : AssetStore {
    val stored = mutableListOf<Pair<String, ByteArray>>()

    override fun put(filename: String, bytes: ByteArray): JSONObject {
        stored.add(filename to bytes)
        return JSONObject().put("asset", "01ASSET${stored.size}").put("filename", filename).put("size", bytes.size)
    }
}
