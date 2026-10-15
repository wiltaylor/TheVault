package dev.wiltaylor.thevault.device

import android.content.ContentResolver
import android.content.ContentUris
import android.provider.Telephony.Mms
import android.webkit.MimeTypeMap
import org.json.JSONArray
import org.json.JSONObject

/** Where captured files go: the local vault's assets. */
fun interface AssetStore {
    /** Store [bytes] as [filename]. Returns `{ asset, filename, size }`. */
    fun put(filename: String, bytes: ByteArray): JSONObject
}

/**
 * Reads received MMS from the Telephony provider as `mms_received` payloads: `{ device_ref,
 * thread_ref, number, number_raw, contact, to, subject, body, attachments, sent }`. A row's key
 * is its `_id`; sent MMS are left out, since no device event carries them.
 *
 * Text parts join into `body`; every other part except the SMIL layout is stored through
 * [assets] and listed in `attachments`. `to` lists the other recipients of a group message,
 * leaving out this phone's own numbers ([selfNumbers], E.164).
 */
class MmsReader(
    private val resolver: ContentResolver,
    private val numbers: PhoneNumbers,
    private val contacts: ContactLookup,
    private val assets: AssetStore,
    private val selfNumbers: () -> Set<String> = { emptySet() },
) {
    private val columns = arrayOf(Mms._ID, Mms.THREAD_ID, Mms.DATE, Mms.DATE_SENT, Mms.SUBJECT, Mms.MESSAGE_BOX)

    /** Received MMS with a key above [after], oldest first. With [range], only those within it (ms). */
    fun since(after: Long, range: LongRange? = null, atMost: Long = Long.MAX_VALUE): List<Captured> {
        val where = StringBuilder("${Mms._ID} > ? AND ${Mms._ID} <= ? AND ${Mms.MESSAGE_BOX} = ${Mms.MESSAGE_BOX_INBOX}")
        val args = mutableListOf(after.toString(), atMost.toString())
        if (range != null) {
            // The provider keeps MMS times in seconds.
            where.append(" AND ${Mms.DATE} >= ? AND ${Mms.DATE} <= ?")
            args += listOf((range.first / 1000).toString(), (range.last / 1000).toString())
        }
        return resolver.rows(Mms.CONTENT_URI, columns, where.toString(), args.toTypedArray(), "${Mms._ID} ASC") { c ->
            val id = c.long(Mms._ID)
            val date = c.long(Mms.DATE) * 1000
            val sent = c.long(Mms.DATE_SENT).takeIf { it > 0 }?.times(1000) ?: date
            Captured("mms_received", payload(id, c.long(Mms.THREAD_ID), c.string(Mms.SUBJECT), sent), id, date)
        }
    }

    /** The highest key, where a cursor starts so that nothing earlier is captured. */
    fun latestKey(): Long =
        resolver.rows(Mms.CONTENT_URI, arrayOf(Mms._ID), order = "${Mms._ID} DESC", limit = 1) { it.long(Mms._ID) }
            .firstOrNull() ?: 0L

    private fun payload(id: Long, thread: Long, subject: String?, sent: Long): JSONObject {
        val addresses = resolver.rows(
            Mms.Addr.getAddrUriForMessage(id.toString()),
            arrayOf(Mms.Addr.ADDRESS, Mms.Addr.TYPE),
        ) { c -> c.int(Mms.Addr.TYPE) to (c.string(Mms.Addr.ADDRESS) ?: "") }
        val from = addresses.firstOrNull { it.first == FROM }?.second
        val self = selfNumbers()
        val to = JSONArray()
        addresses
            .filter { it.first in RECIPIENTS && it.second.isNotEmpty() && it.second != INSERT_ADDRESS_TOKEN }
            .map { it.second }
            .distinct()
            .filter { raw -> raw != from && numbers.normalise(raw).let { it == null || it !in self } }
            .forEach { raw -> to.put(JSONObject().also { Payloads.putNumber(it, raw, numbers, contacts) }) }

        val body = StringBuilder()
        val attachments = JSONArray()
        resolver.rows(
            Mms.Part.CONTENT_URI,
            arrayOf(Mms.Part._ID, Mms.Part.CONTENT_TYPE, Mms.Part.NAME, Mms.Part.FILENAME, Mms.Part.CONTENT_LOCATION, Mms.Part.TEXT),
            "${Mms.Part.MSG_ID} = ?",
            arrayOf(id.toString()),
            "${Mms.Part._ID} ASC",
        ) { c ->
            Part(
                c.long(Mms.Part._ID),
                c.string(Mms.Part.CONTENT_TYPE) ?: "application/octet-stream",
                listOf(Mms.Part.FILENAME, Mms.Part.NAME, Mms.Part.CONTENT_LOCATION).firstNotNullOfOrNull { c.string(it)?.takeIf(String::isNotBlank) },
                c.string(Mms.Part.TEXT),
            )
        }.forEach { part ->
            when {
                part.mime == "application/smil" -> Unit
                part.mime == "text/plain" -> {
                    val text = part.text ?: read(part.id)?.toString(Charsets.UTF_8) ?: ""
                    if (text.isNotEmpty()) {
                        if (body.isNotEmpty()) body.append('\n')
                        body.append(text)
                    }
                }
                else -> read(part.id)?.let { bytes ->
                    val filename = part.filename ?: "part-${part.id}" + extension(part.mime)
                    val stored = assets.put(filename, bytes)
                    attachments.put(
                        JSONObject()
                            .put("asset", stored.getString("asset"))
                            .put("filename", stored.optString("filename", filename))
                            .put("mime", part.mime)
                            .put("size", stored.optLong("size", bytes.size.toLong())),
                    )
                }
            }
        }

        val data = JSONObject()
            .put("device_ref", PREFIX + id)
            .put("thread_ref", thread.toString())
            .put("to", to)
            .put("subject", subject ?: "")
            .put("body", body.toString())
            .put("attachments", attachments)
            .put("sent", Payloads.time(sent))
        Payloads.putNumber(data, from, numbers, contacts)
        return data
    }

    private fun read(partId: Long): ByteArray? =
        resolver.openInputStream(ContentUris.withAppendedId(Mms.Part.CONTENT_URI, partId))?.use { it.readBytes() }

    private fun extension(mime: String): String =
        MimeTypeMap.getSingleton().getExtensionFromMimeType(mime)?.let { ".$it" } ?: ""

    private data class Part(val id: Long, val mime: String, val filename: String?, val text: String?)

    companion object {
        const val PREFIX = "mms:"

        /** PDU address types: from, to, cc and bcc. */
        private const val FROM = 137
        private val RECIPIENTS = setOf(151, 130, 129)

        /** What the provider holds instead of this phone's number on an MMS it sent. */
        private const val INSERT_ADDRESS_TOKEN = "insert-address-token"
    }
}
