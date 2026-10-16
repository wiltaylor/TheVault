package dev.wiltaylor.thevault.device

import android.content.ContentResolver
import android.database.Cursor
import android.provider.Telephony.Sms
import org.json.JSONObject

/**
 * Reads the Telephony provider's SMS as `sms_received` and `sms_sent` payloads: `{ device_ref,
 * thread_ref, number, number_raw, contact, body, sent }`. A row's key is its `_id`; drafts,
 * queued and failed messages are left out.
 */
class SmsReader(
    private val resolver: ContentResolver,
    private val numbers: PhoneNumbers,
    private val contacts: ContactLookup,
) {
    private val columns = arrayOf(Sms._ID, Sms.THREAD_ID, Sms.ADDRESS, Sms.BODY, Sms.DATE, Sms.DATE_SENT, Sms.TYPE)
    private val captured = "${Sms.TYPE} IN (${Sms.MESSAGE_TYPE_INBOX}, ${Sms.MESSAGE_TYPE_SENT})"

    /**
     * Messages with a key above [after], oldest first, at most [limit] of them. With [range], only
     * those sent within it.
     */
    fun since(after: Long, range: LongRange? = null, atMost: Long = Long.MAX_VALUE, limit: Int = Int.MAX_VALUE): List<Captured> {
        val where = StringBuilder("${Sms._ID} > ? AND ${Sms._ID} <= ? AND $captured")
        val args = mutableListOf(after.toString(), atMost.toString())
        if (range != null) {
            where.append(" AND ${Sms.DATE} >= ? AND ${Sms.DATE} <= ?")
            args += listOf(range.first.toString(), range.last.toString())
        }
        return resolver.rows(Sms.CONTENT_URI, columns, where.toString(), args.toTypedArray(), "${Sms._ID} ASC", limit) { c ->
            val (event, data) = payload(c)
            Captured(event, data, c.long(Sms._ID), c.long(Sms.DATE))
        }
    }

    /** The highest key, where a cursor starts so that nothing earlier is captured. */
    fun latestKey(): Long =
        resolver.rows(Sms.CONTENT_URI, arrayOf(Sms._ID), order = "${Sms._ID} DESC", limit = 1) { it.long(Sms._ID) }
            .firstOrNull() ?: 0L

    /** `device::sms_list`: `thread_ref`, `number` (E.164), `since` (a time) and `limit`. */
    fun list(filter: JSONObject?): List<JSONObject> {
        val where = StringBuilder(captured)
        val args = mutableListOf<String>()
        filter.text("thread_ref")?.let { where.append(" AND ${Sms.THREAD_ID} = ?"); args += it }
        Payloads.parseTime(filter.text("since"))?.let { where.append(" AND ${Sms.DATE} >= ?"); args += it.toString() }
        val number = filter.text("number")
        return resolver.rows(Sms.CONTENT_URI, columns, where.toString(), args.toTypedArray(), "${Sms.DATE} DESC") { c ->
            if (number != null && numbers.normalise(c.string(Sms.ADDRESS)) != number) null else payload(c).second
        }.take(filter.limit())
    }

    /** `device::sms_get`, by `device_ref` such as `sms:4412`. */
    fun get(deviceRef: String): JSONObject {
        val id = deviceRef.removePrefix(PREFIX).takeIf { deviceRef.startsWith(PREFIX) }?.toLongOrNull()
            ?: throw IllegalArgumentException("$deviceRef is not an SMS device_ref")
        return resolver.rows(Sms.CONTENT_URI, columns, "${Sms._ID} = ? AND $captured", arrayOf(id.toString()), limit = 1) {
            payload(it).second
        }.firstOrNull() ?: throw NoSuchElementException("no SMS $deviceRef")
    }

    private fun payload(c: Cursor): Pair<String, JSONObject> {
        val received = c.int(Sms.TYPE) == Sms.MESSAGE_TYPE_INBOX
        // A received SMS carries when the sender sent it; some networks leave that out.
        val sent = c.long(Sms.DATE_SENT).takeIf { received && it > 0 } ?: c.long(Sms.DATE)
        val data = JSONObject()
            .put("device_ref", PREFIX + c.long(Sms._ID))
            .put("thread_ref", c.long(Sms.THREAD_ID).toString())
            .put("body", c.string(Sms.BODY) ?: "")
            .put("sent", Payloads.time(sent))
        Payloads.putNumber(data, c.string(Sms.ADDRESS), numbers, contacts)
        return (if (received) "sms_received" else "sms_sent") to data
    }

    companion object {
        const val PREFIX = "sms:"
    }
}
