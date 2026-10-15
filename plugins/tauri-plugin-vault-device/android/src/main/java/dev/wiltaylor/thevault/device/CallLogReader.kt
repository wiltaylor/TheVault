package dev.wiltaylor.thevault.device

import android.content.ContentResolver
import android.database.Cursor
import android.provider.CallLog.Calls
import org.json.JSONObject

/**
 * Reads the call log as `call_ended` payloads: `{ device_ref, direction, number, number_raw,
 * contact, started, duration }`. A row's key is its `_id`. Incoming and answered-elsewhere calls
 * are `in`, outgoing `out`, missed and rejected `missed` with a duration of 0; voicemail and
 * blocked entries are left out.
 */
class CallLogReader(
    private val resolver: ContentResolver,
    private val numbers: PhoneNumbers,
    private val contacts: ContactLookup,
) {
    private val columns = arrayOf(Calls._ID, Calls.NUMBER, Calls.TYPE, Calls.DATE, Calls.DURATION)
    private val captured = "${Calls.TYPE} IN (${Calls.INCOMING_TYPE}, ${Calls.OUTGOING_TYPE}, " +
        "${Calls.MISSED_TYPE}, ${Calls.REJECTED_TYPE}, ${Calls.ANSWERED_EXTERNALLY_TYPE})"

    /** Calls with a key above [after], oldest first. With [range], only those started within it. */
    fun since(after: Long, range: LongRange? = null, atMost: Long = Long.MAX_VALUE): List<Captured> {
        val where = StringBuilder("${Calls._ID} > ? AND ${Calls._ID} <= ? AND $captured")
        val args = mutableListOf(after.toString(), atMost.toString())
        if (range != null) {
            where.append(" AND ${Calls.DATE} >= ? AND ${Calls.DATE} <= ?")
            args += listOf(range.first.toString(), range.last.toString())
        }
        return resolver.rows(Calls.CONTENT_URI, columns, where.toString(), args.toTypedArray(), "${Calls._ID} ASC") { c ->
            Captured("call_ended", payload(c), c.long(Calls._ID), c.long(Calls.DATE))
        }
    }

    /** The highest key, where a cursor starts so that nothing earlier is captured. */
    fun latestKey(): Long =
        resolver.rows(Calls.CONTENT_URI, arrayOf(Calls._ID), order = "${Calls._ID} DESC", limit = 1) { it.long(Calls._ID) }
            .firstOrNull() ?: 0L

    /** `device::calls_list`: `number` (E.164), `since` (a time) and `limit`. */
    fun list(filter: JSONObject?): List<JSONObject> {
        val where = StringBuilder(captured)
        val args = mutableListOf<String>()
        Payloads.parseTime(filter.text("since"))?.let { where.append(" AND ${Calls.DATE} >= ?"); args += it.toString() }
        val number = filter.text("number")
        return resolver.rows(Calls.CONTENT_URI, columns, where.toString(), args.toTypedArray(), "${Calls.DATE} DESC") { c ->
            if (number != null && numbers.normalise(c.string(Calls.NUMBER)) != number) null else payload(c)
        }.take(filter.limit())
    }

    private fun payload(c: Cursor): JSONObject {
        val direction = when (c.int(Calls.TYPE)) {
            Calls.OUTGOING_TYPE -> "out"
            Calls.MISSED_TYPE, Calls.REJECTED_TYPE -> "missed"
            else -> "in"
        }
        val data = JSONObject()
            .put("device_ref", "call:" + c.long(Calls._ID))
            .put("direction", direction)
            .put("started", Payloads.time(c.long(Calls.DATE)))
            .put("duration", if (direction == "missed") 0 else c.long(Calls.DURATION))
        Payloads.putNumber(data, c.string(Calls.NUMBER), numbers, contacts)
        return data
    }
}
