package dev.wiltaylor.thevault.device

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.webkit.JavascriptInterface
import java.util.concurrent.atomic.AtomicBoolean
import org.json.JSONArray
import org.json.JSONObject

/**
 * The home server's notifications as Android notifications (D9). Each one is posted with the
 * server's id as its tag and an id derived from it ([androidId]), so a read, dismiss or clear
 * from anywhere cancels exactly that one. A swipe sends [ACTION_DISMISSED] to
 * [NotificationDismissReceiver], which marks it read on the server; a tap opens the app at its
 * note or page and marks it read too. What this phone reported itself, from `device:<its id>`, is
 * left out: [CaptureProblems] already shows it.
 *
 * All of them are in one group, [GROUP], with this app's own summary ([summary]). Without it,
 * Android groups 3 or more itself, under a summary that cancels the rows on a tap and opens the
 * app with no extras, so they left the shade while still unread on the server (TV-94). A tap on
 * ours cancels nothing, so the rows stay in the shade until each is read or swiped, and opens the
 * app with the bell panel open, the list of what the rows show.
 */
object NotificationMirror {
    const val CHANNEL = "thevault-notifications"

    /** The server id, on the swipe broadcast and on the tap's launch intent. */
    const val EXTRA_ID = "dev.wiltaylor.thevault.notification"

    /** The web client path a tap opens, as the bell would: the note, else a page of the app. */
    const val EXTRA_PATH = "dev.wiltaylor.thevault.path"

    /** On the summary's tap: open the app with the bell panel open ([NotificationPanel]). */
    const val EXTRA_PANEL = "dev.wiltaylor.thevault.panel"

    const val ACTION_DISMISSED = "dev.wiltaylor.thevault.NOTIFICATION_DISMISSED"

    /** The group every mirrored notification is in, with [summary] as its summary. */
    const val GROUP = "dev.wiltaylor.thevault.notifications"

    /** The tag and id of the group summary. [shown] leaves it out by its summary flag. */
    const val SUMMARY_TAG = "thevault-summary"
    const val SUMMARY_ID = 0

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
        .filter { it.notification.flags and Notification.FLAG_GROUP_SUMMARY == 0 }
        .map { it.tag }
        .toSet()

    /**
     * Posts [item], one notification from `GET /api/notifications`, in [GROUP]. The caller then
     * brings the summary in line with [summary].
     */
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
        val builder = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_thevault)
            .setContentTitle(item.optString("title"))
            .setAutoCancel(true)
            .setGroup(GROUP)
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

    /** Cancels the notification for server id [serverId], if it is showing, and updates the summary. */
    fun cancel(context: Context, serverId: String) {
        manager(context).cancel(serverId, androidId(serverId))
        summary(context, shown(context) - serverId)
    }

    /**
     * Posts or updates the group summary for [showing], the server ids in the shade, or cancels
     * it when none is left. It is never cancelled while a row shows, since Android then cancels
     * the rows too. A tap opens the app with the bell panel open ([EXTRA_PANEL]) and does not
     * cancel anything (no auto-cancel); it carries no server id, so nothing is marked read.
     */
    fun summary(context: Context, showing: Set<String>) {
        if (showing.isEmpty()) {
            manager(context).cancel(SUMMARY_TAG, SUMMARY_ID)
            return
        }
        channel(context)
        val open = (
            context.packageManager.getLaunchIntentForPackage(context.packageName)
                ?: Intent(Intent.ACTION_MAIN).setPackage(context.packageName)
            )
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .putExtra(EXTRA_PANEL, true)
        val flags = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        val text = if (showing.size == 1) "1 unread notification" else "${showing.size} unread notifications"
        val summary = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_thevault)
            .setContentTitle("TheVault")
            .setContentText(text)
            .setGroup(GROUP)
            .setGroupSummary(true)
            .setGroupAlertBehavior(Notification.GROUP_ALERT_CHILDREN)
            .setAutoCancel(false)
            .setNumber(showing.size)
            .setContentIntent(PendingIntent.getActivity(context, androidId(SUMMARY_TAG), open, flags))
            .build()
        manager(context).notify(SUMMARY_TAG, SUMMARY_ID, summary)
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
        // What this phone reported itself, its device events that failed for good, it already
        // shows as its own notification ([CaptureProblems]).
        val own = Device.deviceId(context)?.let { "device:$it" }
        val items = (0 until unread.length()).map { unread.getJSONObject(it) }
            .filter { own == null || it.optString("source") != own }
        val ids = items.map { it.getString("id") }.toSet()
        val shown = shown(context)
        for (id in shown - ids) manager(context).cancel(id, androidId(id))
        val showing = shown.intersect(ids).toMutableSet()
        val posted = prefs.getStringSet(POSTED, emptySet())!!.toMutableSet()
        val seen = prefs.getStringSet(SEEN, emptySet())!!.toMutableSet()
        if (!prefs.getBoolean(BASELINE, false)) seen += ids
        // The list is newest first; post oldest first so the newest lands on top.
        for (item in items.asReversed()) {
            val id = item.getString("id")
            if (id !in posted && id !in shown && (id !in seen || id == pushed)) {
                show(context, item)
                posted += id
                showing += id
            }
        }
        // From what this pass did, not from the shade: a post may not be visible there yet.
        summary(context, showing)
        prefs.edit()
            .putStringSet(POSTED, posted.intersect(ids))
            .putStringSet(SEEN, seen.intersect(ids))
            .putBoolean(BASELINE, true)
            .commit()
    }
}

/**
 * The page's side of a tap on the group summary: `window.VaultNotifications.take()` answers
 * whether the bell panel should open, once. The page asks when it starts, for a tap that started
 * the app, and on each [EVENT], which the app sends for a tap while the page is up.
 */
class NotificationPanel {
    private val pending = AtomicBoolean(false)

    /** A summary tap: the page opens the panel when it next asks. */
    fun request() {
        pending.set(true)
    }

    /** Whether the panel should open; true once per [request]. */
    @JavascriptInterface
    fun take(): Boolean = pending.getAndSet(false)

    companion object {
        /** The name the page reaches this by: `window.VaultNotifications`. */
        const val JS_NAME = "VaultNotifications"

        /** The window event that tells a running page to ask [take]. */
        const val EVENT = "vault:open-notifications"

        /** The script that sends [EVENT] to the page. */
        const val SCRIPT = "window.dispatchEvent(new Event('$EVENT'));"
    }
}
