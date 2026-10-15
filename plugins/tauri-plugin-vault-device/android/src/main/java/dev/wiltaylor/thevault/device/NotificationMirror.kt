package dev.wiltaylor.thevault.device

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import org.json.JSONArray
import org.json.JSONObject

/**
 * The home server's notifications as Android notifications (D9). Each one is posted with the
 * server's id as its tag and an id derived from it ([androidId]), so a read, dismiss or clear
 * from anywhere cancels exactly that one. A swipe sends [ACTION_DISMISSED] to
 * [NotificationDismissReceiver], which marks it read on the server; a tap opens the app at its
 * note or page and marks it read too.
 */
object NotificationMirror {
    const val CHANNEL = "thevault-notifications"

    /** The server id, on the swipe broadcast and on the tap's launch intent. */
    const val EXTRA_ID = "dev.wiltaylor.thevault.notification"

    /** The web client path a tap opens, as the bell would: the note, else a page of the app. */
    const val EXTRA_PATH = "dev.wiltaylor.thevault.path"

    const val ACTION_DISMISSED = "dev.wiltaylor.thevault.NOTIFICATION_DISMISSED"

    private const val PREFS = "thevault-notifications"

    /** The server ids ever posted that are still unread, so a swipe is never undone. */
    private const val POSTED = "posted"

    /** What was already unread at the first reconcile: shown only when a push names it. */
    private const val SEEN = "seen"

    /** Set once the first reconcile has noted what was already unread. */
    private const val BASELINE = "baseline"

    /** The Android id for server notification [serverId]: its 32-bit FNV-1a hash. */
    fun androidId(serverId: String): Int {
        var hash = 0x811c9dc5.toInt()
        for (b in serverId.toByteArray(Charsets.UTF_8)) {
            hash = (hash xor (b.toInt() and 0xff)) * 0x01000193
        }
        return hash
    }

    /** Where a tap goes, as the web app's bell decides (`targetOf`), or null for the app itself. */
    fun pathOf(item: JSONObject): String? {
        val note = item.optString("note").takeIf { !item.isNull("note") && it.isNotEmpty() }
        if (note != null) return "/notes/" + android.net.Uri.encode(note)
        val url = item.optString("url").takeIf { !item.isNull("url") } ?: return null
        return url.takeIf { it.startsWith("/") && !it.startsWith("//") }
    }

    private fun manager(context: Context) = context.getSystemService(NotificationManager::class.java)

    private fun channel(context: Context) {
        manager(context).createNotificationChannel(
            NotificationChannel(CHANNEL, "Notifications", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "What your scripts, agents and the server want you to see"
            },
        )
    }

    /** The server ids this app is showing now. */
    fun shown(context: Context): Set<String> = manager(context).activeNotifications
        .filter { it.tag != null && it.id == androidId(it.tag) && it.notification.channelId == CHANNEL }
        .map { it.tag }
        .toSet()

    /** Posts [item], one notification from `GET /api/notifications`. */
    fun show(context: Context, item: JSONObject) {
        channel(context)
        val id = item.getString("id")
        val code = androidId(id)
        val open = (
            context.packageManager.getLaunchIntentForPackage(context.packageName)
                ?: Intent(Intent.ACTION_MAIN).setPackage(context.packageName)
            )
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .putExtra(EXTRA_ID, id)
            .putExtra(EXTRA_PATH, pathOf(item))
        val swipe = Intent(context, NotificationDismissReceiver::class.java)
            .setAction(ACTION_DISMISSED)
            .putExtra(EXTRA_ID, id)
        val flags = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        val body = item.optString("body").takeIf { !item.isNull("body") && it.isNotEmpty() }
        val icon = context.applicationInfo.icon.takeIf { it != 0 } ?: android.R.drawable.ic_dialog_info
        val builder = Notification.Builder(context, CHANNEL)
            .setSmallIcon(icon)
            .setContentTitle(item.optString("title"))
            .setAutoCancel(true)
            .setContentIntent(PendingIntent.getActivity(context, code, open, flags))
            .setDeleteIntent(PendingIntent.getBroadcast(context, code, swipe, flags))
        if (body != null) {
            builder.setContentText(body).setStyle(Notification.BigTextStyle().bigText(body))
        }
        if (item.has("created") && !item.isNull("created")) {
            builder.setWhen(item.getLong("created")).setShowWhen(true)
        }
        manager(context).notify(id, code, builder.build())
    }

    /** Cancels the notification for server id [serverId], if it is showing. */
    fun cancel(context: Context, serverId: String) {
        manager(context).cancel(serverId, androidId(serverId))
    }

    /**
     * Brings what is showing in line with [unread], the server's unread list: a notification
     * read, dismissed or cleared elsewhere is cancelled, and an unread one never posted before is
     * posted. One posted before and gone from the shade, swiped while its read is still on its
     * way, stays gone. The first time, what is already unread is only noted as seen, so pairing
     * does not flood the shade; a push naming one of those, as [pushed], still shows it.
     */
    fun reconcile(context: Context, unread: JSONArray, pushed: String? = null) {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val items = (0 until unread.length()).map { unread.getJSONObject(it) }
        val ids = items.map { it.getString("id") }.toSet()
        val shown = shown(context)
        for (id in shown - ids) cancel(context, id)
        val posted = prefs.getStringSet(POSTED, emptySet())!!.toMutableSet()
        val seen = prefs.getStringSet(SEEN, emptySet())!!.toMutableSet()
        if (!prefs.getBoolean(BASELINE, false)) seen += ids
        // The list is newest first; post oldest first so the newest lands on top.
        for (item in items.asReversed()) {
            val id = item.getString("id")
            if (id !in posted && id !in shown && (id !in seen || id == pushed)) {
                show(context, item)
                posted += id
            }
        }
        prefs.edit()
            .putStringSet(POSTED, posted.intersect(ids))
            .putStringSet(SEEN, seen.intersect(ids))
            .putBoolean(BASELINE, true)
            .commit()
    }
}
