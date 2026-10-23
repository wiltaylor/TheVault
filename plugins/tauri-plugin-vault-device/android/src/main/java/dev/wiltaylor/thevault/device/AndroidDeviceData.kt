package dev.wiltaylor.thevault.device

import android.Manifest
import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Handler
import android.os.HandlerThread
import android.telephony.SmsManager
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.json.JSONArray
import org.json.JSONObject

/**
 * The device module's view of the phone (D23): reads through the same readers capture uses, and
 * contact writes, SMS sending and notifications through Android. The core enforces the device's
 * switches before it gets here; Android enforces its permissions, and a missing one comes back as
 * an error the script receives.
 */
class AndroidDeviceData(
    private val context: Context,
    private val capture: DeviceCapture,
    private val noted: (String) -> Boolean = { VaultCore.noted(it) == "true" },
    private val sentTimeoutMs: Long = SENT_TIMEOUT_MS,
    private val rowWaitMs: Long = ROW_WAIT_MS,
) : DeviceBridge {
    override fun call(method: String, args: String): String = try {
        val a = JSONObject(args)
        val ok: Any = when (method) {
            "sms_list" -> JSONArray(capture.sms.list(a.optJSONObject("filter")))
            "sms_get" -> capture.sms.get(a.getString("device_ref"))
            "calls_list" -> JSONArray(capture.calls.list(a.optJSONObject("filter")))
            "contacts_list" -> JSONArray(capture.contacts.list(a.optJSONObject("filter")))
            "contacts_get" -> capture.contacts.get(a.getString("contact_id"))
            "contacts_upsert" -> capture.contacts.upsert(a.getJSONObject("contact"))
            "contacts_delete" -> JSONObject().also { capture.contacts.delete(a.getString("contact_id")) }
            "contacts_forget_links" -> JSONObject().put("removed", capture.contacts.forgetOtherVaults(a.getString("vault_id")))
            "sms_send" -> sendSms(a.getString("to"), a.getString("body"))
            "show_notification" -> JSONObject().also {
                showNotification(a.optString("title"), a.optString("body"), a.optString("note"))
            }
            "history" -> history(a)
            else -> throw IllegalArgumentException("the device has no $method")
        }
        JSONObject().put("ok", ok).toString()
    } catch (e: SecurityException) {
        err("Android has not given TheVault the permission $method needs: ${e.message}")
    } catch (e: Exception) {
        err(e.message ?: e.toString())
    }

    private fun err(message: String) = JSONObject().put("err", message).toString()

    /**
     * One page of an import (`DeviceData::history`): `{ kind, since, until, after, limit }` in,
     * `{ events, next, missing_permission }` out. A kind whose permission Android has not granted
     * answers nothing but the permission, so the import can say what to allow.
     */
    private fun history(a: JSONObject): JSONObject {
        val kind = a.getString("kind")
        val permission = DeviceCapture.HISTORY_PERMISSIONS[kind]
            ?: throw IllegalArgumentException("$kind cannot be imported")
        if (context.checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED) {
            return DeviceCapture.HistoryPage(emptyList(), null, permission).toJson()
        }
        fun time(name: String, otherwise: Long) = if (a.isNull(name)) otherwise else a.getLong(name)
        val range = time("since", 0L)..time("until", Long.MAX_VALUE)
        val after = if (a.isNull("after")) null else a.getLong("after")
        return capture.history(kind, range, after, a.optInt("limit", DeviceCapture.HISTORY_PAGE), noted).toJson()
    }

    /**
     * Sends [body] to [to] (D23, outbox X6), then waits up to [sentTimeoutMs] for Android's sent
     * broadcast of every part. Sent, it answers `{ device_ref }`: the row Android wrote to the SMS
     * provider for the message (the newest sent row to [to] with [body]), read back as capture
     * reads it, or null when no such row appears within [rowWaitMs]. A part Android could not send
     * throws with its SmsManager result code, and so does no answer in time, since the message may
     * or may not have gone: the command fails rather than have a retry send it twice.
     */
    private fun sendSms(to: String, body: String): JSONObject {
        if (context.checkSelfPermission(Manifest.permission.SEND_SMS) != PackageManager.PERMISSION_GRANTED) {
            throw SecurityException("SEND_SMS")
        }
        val sms = context.getSystemService(SmsManager::class.java)
        // Any body this short fits one SMS, whatever its alphabet; a longer one goes in parts.
        val parts = if (body.length <= SINGLE_SMS) arrayListOf(body) else sms.divideMessage(body)
        val before = capture.sms.latestKey()
        val sent = SentWaiter(context, parts.size)
        try {
            val intents = ArrayList(parts.indices.map { sent.intent(it) })
            if (parts.size > 1) {
                sms.sendMultipartTextMessage(to, null, parts, intents, null)
            } else {
                sms.sendTextMessage(to, null, body, intents[0], null)
            }
            val code = sent.await(sentTimeoutMs)
                ?: throw IllegalStateException(
                    "Android did not say within ${sentTimeoutMs / 1000} s whether the SMS went. " +
                        "Check the phone's messages before you send it again.",
                )
            if (code != Activity.RESULT_OK) throw IllegalStateException("SmsManager result code $code (${resultName(code)})")
        } finally {
            sent.close()
        }
        return JSONObject().put("device_ref", sentRow(to, body, before) ?: JSONObject.NULL)
    }

    /** The sent row of the message just sent, which Android writes as it reports the send. */
    private fun sentRow(to: String, body: String, before: Long): String? {
        val until = System.currentTimeMillis() + rowWaitMs
        while (true) {
            capture.sms.latestSent(to, body, before)?.let { return it }
            if (System.currentTimeMillis() >= until) return null
            Thread.sleep(ROW_POLL_MS)
        }
    }

    /**
     * Android's sent broadcasts for one message of [parts] parts: one [PendingIntent] per part, on
     * an action of this send's own, received on a thread of its own, since the caller blocks.
     */
    private class SentWaiter(private val context: Context, parts: Int) {
        private val action = "$SENT_ACTION.${UUID.randomUUID()}"
        private val left = CountDownLatch(parts)
        private val failed = AtomicInteger(Activity.RESULT_OK)
        private val thread = HandlerThread("sms-sent").apply { start() }
        private val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                if (resultCode != Activity.RESULT_OK) failed.compareAndSet(Activity.RESULT_OK, resultCode)
                left.countDown()
            }
        }

        init {
            context.registerReceiver(receiver, IntentFilter(action), null, Handler(thread.looper), Context.RECEIVER_NOT_EXPORTED)
        }

        fun intent(part: Int): PendingIntent = PendingIntent.getBroadcast(
            context,
            part,
            Intent(action).setPackage(context.packageName),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_ONE_SHOT,
        )

        /** The first failing part's result code, else [Activity.RESULT_OK]; null when not every part reported in time. */
        fun await(timeoutMs: Long): Int? = if (left.await(timeoutMs, TimeUnit.MILLISECONDS)) failed.get() else null

        fun close() {
            context.unregisterReceiver(receiver)
            thread.quitSafely()
        }
    }

    private fun showNotification(title: String, body: String, note: String) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Device scripts", NotificationManager.IMPORTANCE_DEFAULT),
        )
        val open = context.packageManager.getLaunchIntentForPackage(context.packageName)?.apply {
            if (note.isNotEmpty()) data = Uri.parse("thevault://note/$note")
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val builder = android.app.Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_thevault)
            .setContentTitle(title)
            .setAutoCancel(true)
        if (body.isNotEmpty()) builder.setContentText(body)
        open?.let {
            builder.setContentIntent(
                PendingIntent.getActivity(context, note.hashCode(), it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT),
            )
        }
        manager.notify("device-script", (title + note + System.nanoTime()).hashCode(), builder.build())
    }

    companion object {
        const val CHANNEL = "device-scripts"

        /** The action prefix of the sent broadcasts; each send adds an id of its own. */
        const val SENT_ACTION = "dev.wiltaylor.thevault.SMS_SENT"

        /** How long a send waits for Android to say whether the SMS went: 60 s (outbox X6). */
        const val SENT_TIMEOUT_MS = 60_000L

        /** How long a sent message's row may take to appear in the SMS provider. */
        const val ROW_WAIT_MS = 5_000L

        private const val ROW_POLL_MS = 100L

        /** The longest body that is always one SMS: 70 characters in UCS-2, 160 in GSM 7-bit. */
        private const val SINGLE_SMS = 70

        /** What an SmsManager result code means, as far as the error says. */
        fun resultName(code: Int): String = when (code) {
            SmsManager.RESULT_ERROR_GENERIC_FAILURE -> "generic failure"
            SmsManager.RESULT_ERROR_RADIO_OFF -> "radio off"
            SmsManager.RESULT_ERROR_NULL_PDU -> "no PDU"
            SmsManager.RESULT_ERROR_NO_SERVICE -> "no service"
            SmsManager.RESULT_ERROR_LIMIT_EXCEEDED -> "sending limit reached"
            SmsManager.RESULT_ERROR_SHORT_CODE_NOT_ALLOWED -> "short code not allowed"
            SmsManager.RESULT_ERROR_SHORT_CODE_NEVER_ALLOWED -> "short code never allowed"
            SmsManager.RESULT_RADIO_NOT_AVAILABLE -> "radio not available"
            SmsManager.RESULT_NETWORK_REJECT -> "the network refused it"
            SmsManager.RESULT_INVALID_SMS_FORMAT -> "invalid SMS format"
            SmsManager.RESULT_NETWORK_ERROR -> "network error"
            SmsManager.RESULT_NO_DEFAULT_SMS_APP -> "no default SMS app"
            else -> "see SmsManager"
        }
    }
}
