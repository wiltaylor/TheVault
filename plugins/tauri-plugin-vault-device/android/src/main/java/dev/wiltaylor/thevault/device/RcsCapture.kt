package dev.wiltaylor.thevault.device

import android.app.Notification
import android.content.ContentResolver
import android.content.SharedPreferences
import android.provider.Telephony.Mms
import android.provider.Telephony.Sms
import android.util.Log
import androidx.core.app.NotificationCompat
import org.json.JSONObject

/**
 * Reads Google Messages' notifications as incoming messages (D13). No public RCS API exists, so
 * this is the only way RCS reaches the vault, and it is lossy: a message read while its chat is
 * open posts no notification, a sender is a display name rather than a number, attachments are
 * not available, and messages this phone sends are not seen.
 *
 * Google Messages posts a `MessagingStyle` notification per conversation and reposts it with
 * each new message, carrying the conversation's recent messages: each has its text, its time and
 * its sender as a `Person`. Messages the user sent carry the user's own `Person`, or none.
 */
object GoogleMessages {
    const val PACKAGE = "com.google.android.apps.messaging"

    /** One incoming message from a notification, as an `rcs_received` payload's parts. */
    data class Incoming(
        val deviceRef: String,
        val threadRef: String,
        val sender: String,
        val conversation: String?,
        val body: String,
        val sentMs: Long,
    ) {
        /** The `rcs_received` payload (`docs/scripts.md` "Device events"). */
        fun payload(): JSONObject = JSONObject()
            .put("device_ref", deviceRef)
            .put("thread_ref", threadRef)
            .put("sender", sender)
            .put("conversation", conversation ?: JSONObject.NULL)
            .put("body", body)
            .put("sent", Payloads.time(sentMs))
            .put("partial", true)
    }

    /**
     * The incoming messages a notification posted by [packageName] under [key] carries, oldest
     * first. Anything that is not a Google Messages conversation notification yields none.
     *
     * A conversation is the notification's shortcut id, which Google Messages sets to its
     * conversation id, or the notification key when it has none; a group chat is one
     * conversation, titled by the notification's conversation title. A message is the
     * notification key and the message's time, which stay the same each time it is reposted.
     */
    fun parse(packageName: String, key: String, notification: Notification): List<Incoming> {
        if (packageName != PACKAGE) return emptyList()
        if (notification.flags and Notification.FLAG_GROUP_SUMMARY != 0) return emptyList()
        val style = NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(notification)
            ?: return emptyList()
        val self = self(notification, style)
        val thread = notification.shortcutId?.takeIf { it.isNotEmpty() } ?: key
        val conversation = style.conversationTitle?.toString()?.takeIf { style.isGroupConversation && it.isNotBlank() }
        return style.messages.mapNotNull { m ->
            val person = m.person ?: return@mapNotNull null
            val sender = person.name?.toString()?.trim()?.takeIf { it.isNotEmpty() } ?: return@mapNotNull null
            if (person.key?.let { it in self.keys } ?: (sender in self.names)) return@mapNotNull null
            val body = m.text?.toString()?.takeIf { it.isNotBlank() } ?: return@mapNotNull null
            if (m.timestamp <= 0) return@mapNotNull null
            Incoming("$PREFIX$key:${m.timestamp}", thread, sender, conversation, body, m.timestamp)
        }.sortedBy { it.sentMs }
    }

    /** Who the user is in a notification: by `Person` key, or by name when a sender has no key. */
    private data class Self(val keys: Set<String>, val names: Set<String>)

    /**
     * The user's own identity. A notification built by the platform's `MessagingStyle` holds it
     * as `android.messagingUser`, which androidx reads only as a name; one built by androidx's
     * holds it as `android.messagingStyleUser`. Either way, a message the user sent is theirs.
     */
    private fun self(notification: Notification, style: NotificationCompat.MessagingStyle): Self {
        val platform = notification.extras.getParcelable(Notification.EXTRA_MESSAGING_PERSON, android.app.Person::class.java)
        val keys = listOfNotNull(style.user.key, platform?.key).toSet()
        val names = listOfNotNull(style.user.name, platform?.name)
            .map { it.toString().trim() }
            .filter { it.isNotEmpty() }
            .toSet()
        return Self(keys, names)
    }

    const val PREFIX = "rcs:"
}

/**
 * The `device_ref`s of messages already handed on, so a reposted notification's earlier messages
 * are not captured again. Keeps the most recent [LIMIT]; the handlers de-duplicate on
 * `device_ref` too, for anything older.
 */
class SeenRefs(private val prefs: SharedPreferences) {
    @Synchronized
    fun add(ref: String): Boolean {
        val refs = load()
        if (ref in refs) return false
        refs.addLast(ref)
        while (refs.size > LIMIT) refs.removeFirst()
        prefs.edit().putString(KEY, refs.joinToString("\n")).commit()
        return true
    }

    private fun load(): ArrayDeque<String> =
        ArrayDeque(prefs.getString(KEY, null)?.split('\n')?.filter { it.isNotEmpty() }.orEmpty())

    companion object {
        private const val KEY = "rcs.seen"
        const val LIMIT = 500
    }
}

/**
 * Captures RCS from Google Messages notifications into the [sink] as `rcs_received` (D13).
 *
 * Google Messages is the default SMS app too, so it posts the same notifications for SMS and MMS,
 * which capture already reads from the Telephony provider. A message is dropped as one of those
 * when the provider holds a received SMS or MMS with the same text within [WINDOW_MS] of its
 * time. The notification's conversation and the provider's thread have different ids, and the
 * notification names a sender where the provider has a number, so text and time are what match.
 * Without the SMS permission nothing can be matched, and nothing is captured from the provider
 * either, so every message is kept.
 */
class RcsCapture(
    private val resolver: ContentResolver,
    private val sink: EventSink,
    private val seen: SeenRefs,
) {
    /** Capture the new messages of a notification. Returns how many events were captured. */
    fun capture(packageName: String, key: String, notification: Notification): Int {
        var n = 0
        for (m in GoogleMessages.parse(packageName, key, notification)) {
            if (!seen.add(m.deviceRef)) continue
            if (fromProvider(m)) continue
            sink.enqueue("rcs_received", m.payload(), System.currentTimeMillis())
            n++
        }
        if (n > 0) sink.flush()
        return n
    }

    /** Whether the Telephony provider holds [m] as a received SMS or MMS. */
    private fun fromProvider(m: GoogleMessages.Incoming): Boolean = try {
        val text = same(m.body)
        smsBodies(m.sentMs).any { same(it) == text } || mmsBodies(m.sentMs).any { same(it) == text }
    } catch (e: SecurityException) {
        Log.i(TAG, "cannot match RCS against SMS without permission: ${e.message}")
        false
    }

    private fun smsBodies(at: Long): List<String> {
        val from = (at - WINDOW_MS).toString()
        val to = (at + WINDOW_MS).toString()
        return resolver.rows(
            Sms.CONTENT_URI,
            arrayOf(Sms.BODY),
            "${Sms.TYPE} = ${Sms.MESSAGE_TYPE_INBOX} AND ((${Sms.DATE} >= ? AND ${Sms.DATE} <= ?) OR (${Sms.DATE_SENT} >= ? AND ${Sms.DATE_SENT} <= ?))",
            arrayOf(from, to, from, to),
        ) { it.string(Sms.BODY) }
    }

    private fun mmsBodies(at: Long): List<String> {
        // The provider keeps MMS times in seconds.
        val ids = resolver.rows(
            Mms.CONTENT_URI,
            arrayOf(Mms._ID),
            "${Mms.MESSAGE_BOX} = ${Mms.MESSAGE_BOX_INBOX} AND ${Mms.DATE} >= ? AND ${Mms.DATE} <= ?",
            arrayOf(((at - WINDOW_MS) / 1000).toString(), ((at + WINDOW_MS) / 1000 + 1).toString()),
        ) { it.long(Mms._ID) }
        return ids.map { id ->
            resolver.rows(
                Mms.Part.CONTENT_URI,
                arrayOf(Mms.Part.TEXT),
                "${Mms.Part.MSG_ID} = ? AND ${Mms.Part.CONTENT_TYPE} = 'text/plain'",
                arrayOf(id.toString()),
                "${Mms.Part._ID} ASC",
            ) { it.string(Mms.Part.TEXT) }.joinToString("\n")
        }
    }

    /** Text as compared: whitespace runs collapsed, ends trimmed. */
    private fun same(text: String) = text.trim().replace(SPACE, " ")

    companion object {
        private const val TAG = "VaultRcsCapture"
        private val SPACE = Regex("\\s+")

        /** How far apart a notification's time and the provider's may be for one message. */
        const val WINDOW_MS = 120_000L
    }
}
