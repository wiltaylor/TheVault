package dev.wiltaylor.thevault.device

import android.content.ContentResolver
import android.content.SharedPreferences
import android.util.Log
import org.json.JSONObject

/** Where captured device events go: the device event queue, through JNI in the app. */
interface EventSink {
    /** Store [event] with payload [data], captured at [atMs]. Durable once this returns. */
    fun enqueue(event: String, data: JSONObject, atMs: Long)

    /** Events were stored: run their handlers soon. */
    fun flush()
}

/**
 * How far capture has read each provider. A key is a row id (SMS, MMS, calls) or a time in
 * milliseconds (contact changes and deletions). `<kind>.start` holds where capture started, the
 * ceiling of what a first-run import reads.
 */
class Cursors(private val prefs: SharedPreferences) {
    fun get(kind: String): Long? = if (prefs.contains(kind)) prefs.getLong(kind, 0L) else null

    fun set(kind: String, key: Long) {
        // Committed before the next event is read, so a crash re-reads at most one row.
        prefs.edit().putLong(kind, key).commit()
    }

    fun start(kind: String): Long? = get("$kind.start")

    fun begin(kind: String, key: Long) {
        prefs.edit().putLong(kind, key).putLong("$kind.start", key).commit()
    }

    companion object {
        const val SMS = "sms"
        const val MMS = "mms"
        const val CALLS = "calls"
        const val CONTACTS = "contacts"
        const val CONTACTS_DELETED = "contacts_deleted"
        val ALL = listOf(SMS, MMS, CALLS, CONTACTS, CONTACTS_DELETED)
    }
}

/**
 * Captures device events from the phone's providers into the [sink] (D7).
 *
 * Live capture (a receiver, a content observer, a content-URI job) and the periodic catch-up both
 * read each provider from its stored cursor, and advance the cursor past every row they hand on,
 * so a row is captured once whichever path reads it first. The handlers de-duplicate on
 * `device_ref` as well, for the rare row read twice across a crash.
 *
 * Capture starts at the providers' newest rows ([begin]): history is only read by
 * [importHistory], which the user turns on at first run with a date range.
 */
class DeviceCapture(
    resolver: ContentResolver,
    numbers: PhoneNumbers,
    private val sink: EventSink,
    private val cursors: Cursors,
    assets: AssetStore,
    selfNumbers: () -> Set<String> = { emptySet() },
) {
    private val contactLookup = ContactLookup(resolver)
    val sms = SmsReader(resolver, numbers, contactLookup)
    val mms = MmsReader(resolver, numbers, contactLookup, assets, selfNumbers)
    val calls = CallLogReader(resolver, numbers, contactLookup)
    val contacts = ContactsReader(resolver, numbers)

    private val latest: Map<String, () -> Long> = mapOf(
        Cursors.SMS to sms::latestKey,
        Cursors.MMS to mms::latestKey,
        Cursors.CALLS to calls::latestKey,
        Cursors.CONTACTS to contacts::latestChange,
        Cursors.CONTACTS_DELETED to contacts::latestDeletion,
    )

    /**
     * Start capture at each provider's newest row, for every provider not started yet. A provider
     * whose permission is missing is left to start when it is granted.
     */
    @Synchronized
    fun begin() {
        for ((kind, newest) in latest) {
            if (cursors.get(kind) != null) continue
            readable(kind) { cursors.begin(kind, newest()) }
        }
    }

    /** New SMS. Returns how many events were captured. */
    fun captureSms(): Int = capture(Cursors.SMS) { sms.since(it) }

    /** New received MMS. */
    fun captureMms(): Int = capture(Cursors.MMS) { mms.since(it) }

    /** New calls. */
    fun captureCalls(): Int = capture(Cursors.CALLS) { calls.since(it) }

    /** Contacts changed or deleted since the last read. */
    fun captureContacts(): Int =
        capture(Cursors.CONTACTS) { contacts.changedSince(it) } +
            capture(Cursors.CONTACTS_DELETED) { contacts.deletedSince(it) }

    /** Have the sink run what was captured. */
    fun flush() = sink.flush()

    /** Every provider since its cursor: the periodic catch-up. */
    fun catchUp(): Int {
        begin()
        val n = captureSms() + captureMms() + captureCalls() + captureContacts()
        if (n > 0) sink.flush()
        return n
    }

    /**
     * The first-run import: SMS, MMS and calls from within [fromMs]..[toMs], and contacts changed
     * within it, up to where live capture started, so nothing is captured twice.
     */
    fun importHistory(fromMs: Long, toMs: Long): Int {
        begin()
        val range = fromMs..toMs
        var n = 0
        fun from(kind: String, read: (Long) -> List<Captured>) {
            val ceiling = cursors.start(kind) ?: return
            readable(kind) { read(ceiling).forEach { hand(it); n++ } }
        }
        from(Cursors.SMS) { sms.since(0, range, it) }
        from(Cursors.MMS) { mms.since(0, range, it) }
        from(Cursors.CALLS) { calls.since(0, range, it) }
        from(Cursors.CONTACTS) { contacts.changedSince(maxOf(fromMs - 1, 0), minOf(toMs, it)) }
        if (n > 0) sink.flush()
        return n
    }

    private fun capture(kind: String, read: (Long) -> List<Captured>): Int = synchronized(lockOf(kind)) {
        val after = cursors.get(kind) ?: run {
            begin()
            return 0
        }
        var n = 0
        readable(kind) {
            for (row in read(after)) {
                hand(row)
                cursors.set(kind, row.key)
                n++
            }
        }
        n
    }

    private fun hand(row: Captured) = sink.enqueue(row.event, row.data, System.currentTimeMillis())

    private inline fun readable(kind: String, read: () -> Unit) {
        try {
            read()
        } catch (e: SecurityException) {
            Log.i(TAG, "no permission to read $kind yet: ${e.message}")
        }
    }

    private val locks = Cursors.ALL.associateWith { Any() }

    private fun lockOf(kind: String): Any = locks.getValue(kind)

    companion object {
        private const val TAG = "VaultDeviceCapture"
    }
}
