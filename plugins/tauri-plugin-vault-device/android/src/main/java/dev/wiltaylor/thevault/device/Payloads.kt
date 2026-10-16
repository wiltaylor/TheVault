package dev.wiltaylor.thevault.device

import android.content.ContentResolver
import android.database.Cursor
import android.net.Uri
import org.json.JSONArray
import org.json.JSONObject
import java.time.Instant
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.temporal.ChronoUnit

/**
 * One row a reader turned into a device event: its name and payload (`docs/scripts.md` "Device
 * events"), the row's key that the capture cursor advances to, and when it happened. A [known]
 * row, read for an import, already has a note, and its payload is its `device_ref` alone.
 */
data class Captured(val event: String, val data: JSONObject, val key: Long, val atMs: Long, val known: Boolean = false)

/** Device event payload conventions shared by every reader. */
object Payloads {
    /** [ms] as a payload time: RFC 3339 with the phone's offset, to the second. */
    fun time(ms: Long, zone: ZoneId = ZoneId.systemDefault()): String =
        OffsetDateTime.ofInstant(Instant.ofEpochMilli(ms), zone)
            .truncatedTo(ChronoUnit.SECONDS)
            .format(DateTimeFormatter.ISO_OFFSET_DATE_TIME)

    /** A payload time back in milliseconds, or null when [value] is not one. */
    fun parseTime(value: String?): Long? = try {
        if (value.isNullOrEmpty()) null else OffsetDateTime.parse(value).toInstant().toEpochMilli()
    } catch (_: Exception) {
        null
    }

    /** The `number` and `number_raw` members of a payload, and its `contact`, for [raw]. */
    fun putNumber(target: JSONObject, raw: String?, numbers: PhoneNumbers, contacts: ContactLookup) {
        target.put("number", numbers.normalise(raw) ?: JSONObject.NULL)
        target.put("number_raw", raw ?: "")
        target.put("contact", contacts.find(raw) ?: JSONObject.NULL)
    }

    fun list(values: Iterable<Any>): JSONArray = JSONArray().apply { values.forEach { put(it) } }
}

/** Every row of a query as a list, or an empty list when the provider answers null. */
inline fun <T> ContentResolver.rows(
    uri: Uri,
    projection: Array<String>,
    selection: String? = null,
    args: Array<String>? = null,
    order: String? = null,
    limit: Int = Int.MAX_VALUE,
    read: (Cursor) -> T?,
): List<T> {
    val out = ArrayList<T>()
    query(uri, projection, selection, args, order)?.use { c ->
        while (out.size < limit && c.moveToNext()) {
            read(c)?.let { out.add(it) }
        }
    }
    return out
}

fun Cursor.string(column: String): String? =
    getColumnIndex(column).takeIf { it >= 0 && !isNull(it) }?.let { getString(it) }

fun Cursor.long(column: String): Long =
    getColumnIndex(column).takeIf { it >= 0 && !isNull(it) }?.let { getLong(it) } ?: 0L

fun Cursor.int(column: String): Int = long(column).toInt()

/** A device module filter's `limit`, or [default]. */
fun JSONObject?.limit(default: Int = 100): Int =
    this?.optInt("limit", default)?.takeIf { it > 0 } ?: default

/** A device module filter's string member, or null when absent or null. */
fun JSONObject?.text(name: String): String? =
    this?.takeIf { it.has(name) && !it.isNull(name) }?.optString(name)?.takeIf { it.isNotEmpty() }
