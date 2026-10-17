package dev.wiltaylor.thevault.device

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject

/**
 * Device events whose handler failed on every pass, announced on the phone (ticket 88). After a
 * pass, [announce] asks the core for the failures it has not announced yet and shows one
 * notification, "A message could not be saved", which a tap opens at Settings > Devices > This
 * device, where "Capture problems" lists them with Retry and Dismiss. A burst of failures is one
 * notification: while it is showing, each new failure updates it with the running count.
 *
 * The home server hears of each failure from the core's next sync round, which adds one
 * notification under the bell; [NotificationMirror] leaves that one out of the shade, since this
 * one already says it.
 */
object CaptureProblems {
    const val CHANNEL = "thevault-capture-problems"

    /** The one notification's id and its group, so a burst stays one notification. */
    const val NOTIFICATION_ID = 0x7a88
    const val GROUP = "dev.wiltaylor.thevault.CAPTURE_PROBLEMS"

    const val TITLE = "A message could not be saved"

    /** Where a tap opens the app: Settings > Devices > This device. */
    const val PATH = "/settings/devices#this-device"

    private const val PREFS = "thevault-capture-problems"

    /** How many failures the notification showing now counts. */
    private const val COUNT = "count"

    private const val TAG = "VaultCaptureProblems"

    private fun manager(context: Context) = context.getSystemService(NotificationManager::class.java)

    /**
     * Announce the device events that failed for good since the last call, if any. Answers how
     * many there were, so the caller can have the core report them to the home server soon.
     */
    fun announce(context: Context): Int {
        val fresh = try {
            JSONArray(VaultCore.newFailures())
        } catch (e: IllegalStateException) {
            // The device runtime is not open yet: nothing has run, so nothing failed.
            return 0
        }
        if (fresh.length() > 0) show(context, fresh)
        return fresh.length()
    }

    /** Whether the notification is showing now. */
    fun showing(context: Context): Boolean =
        manager(context).activeNotifications.any { it.id == NOTIFICATION_ID && it.notification.channelId == CHANNEL }

    /** One line for failure [e]: what, from whom, and the handler's message. */
    fun describe(e: JSONObject): String {
        val what = when (e.optString("event")) {
            "call_ended" -> "A call"
            "contact_changed", "contact_deleted" -> "A contact change"
            "share_received" -> "A share"
            else -> "A message"
        }
        val who = e.optString("who").takeIf { !e.isNull("who") && it.isNotBlank() }
        val message = e.optString("message").takeIf { !e.isNull("message") && it.isNotBlank() }
        return buildString {
            append(what)
            if (who != null) append(" from ").append(who)
            append(" was not saved")
            if (message != null) append(": ").append(message)
        }
    }

    /** Show [failures], adding them to the notification when it is still showing. */
    fun show(context: Context, failures: JSONArray) {
        if (failures.length() == 0) return
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val before = if (showing(context)) prefs.getInt(COUNT, 0) else 0
        val count = before + failures.length()
        prefs.edit().putInt(COUNT, count).apply()
        manager(context).createNotificationChannel(
            NotificationChannel(CHANNEL, "Capture problems", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "A message, call or contact change that could not be saved as a note"
            },
        )
        val newest = failures.getJSONObject(failures.length() - 1)
        val text = if (count == 1) {
            describe(newest)
        } else {
            "$count messages, calls or contact changes were not saved. Open TheVault to retry or dismiss them."
        }
        val open = (
            context.packageManager.getLaunchIntentForPackage(context.packageName)
                ?: Intent(Intent.ACTION_MAIN).setPackage(context.packageName)
            )
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .putExtra(NotificationMirror.EXTRA_PATH, PATH)
        val flags = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        val notification = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_thevault)
            .setContentTitle(TITLE)
            .setContentText(text)
            .setStyle(Notification.BigTextStyle().bigText(text))
            .setNumber(count)
            .setGroup(GROUP)
            .setGroupSummary(true)
            .setOnlyAlertOnce(true)
            .setAutoCancel(true)
            .setContentIntent(PendingIntent.getActivity(context, NOTIFICATION_ID, open, flags))
            .build()
        try {
            manager(context).notify(NOTIFICATION_ID, notification)
        } catch (e: SecurityException) {
            // Notifications are not allowed: This device still lists the problems.
            Log.w(TAG, "capture problems not shown", e)
        }
    }
}
