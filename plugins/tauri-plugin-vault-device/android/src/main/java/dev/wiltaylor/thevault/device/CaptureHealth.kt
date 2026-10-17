package dev.wiltaylor.thevault.device

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject

/**
 * Whether capture can work on this phone (ticket 91): each capture kind's permission, Android's
 * restricted settings and battery optimisation, and whether the background work runs. On the
 * live phone a reinstall left SMS and calls uncaptured and device commands pending, with no sign
 * of why: this says why.
 *
 * [check] answers the health the Capture health panel of Settings > Devices > This device shows.
 * [refresh] checks, hands the core the report it sends to the home server with its next settings
 * sync, and raises one notification, which opens the panel, for a persistent problem: a
 * permission missing, battery optimisation on, or background work stalled for [STALE_MS]. Each
 * problem raises it at most once a day.
 */
object CaptureHealth {
    const val WORKING = "working"
    const val PERMISSION_MISSING = "permission_missing"
    const val RESTRICTED = "restricted"
    const val NOT_SET_UP = "not_set_up"

    /** The system screens a problem's button opens ([fixIntent]). */
    const val FIX_APP_DETAILS = "app_details"
    const val FIX_NOTIFICATION_ACCESS = "notification_access"
    const val FIX_BATTERY = "battery"
    const val FIX_CAPTURE_SETUP = "capture_setup"
    val FIXES = listOf(FIX_APP_DETAILS, FIX_NOTIFICATION_ACCESS, FIX_BATTERY, FIX_CAPTURE_SETUP)

    /** The background work whose liveness is watched. */
    const val CATCH_UP = "catch_up"
    const val COMMANDS = "commands"

    /** How long background work may go without running before it counts as stalled. */
    const val STALE_MS = 2 * 60 * 60 * 1000L

    /** How often one problem may raise the notification. */
    const val NOTIFY_EVERY_MS = 24 * 60 * 60 * 1000L

    const val CHANNEL = "thevault-capture-health"
    const val NOTIFICATION_ID = 0x7a91
    const val TITLE = "TheVault is not capturing everything"

    /** Where a tap opens the app: the Capture health panel of This device. */
    const val PATH = "/settings/devices#capture-health"

    const val SAMSUNG_LINE =
        "On a Samsung phone, also keep TheVault out of Sleeping apps and Deep sleeping apps: " +
            "Settings > Battery > Background usage limits."

    private const val PREFS = "thevault-capture-health"
    private const val TAG = "VaultCaptureHealth"

    /** The app stores whose installs Android's restricted settings leave alone. */
    private val STORES = setOf("com.android.vending")

    /**
     * One kind of capture: what it needs, and from which Android release restricted settings
     * block that for an app installed from outside a store.
     */
    data class Kind(
        val id: String,
        val label: String,
        val permissions: List<String>,
        val listener: Boolean = false,
        val restrictedFromSdk: Int,
    )

    val KINDS = listOf(
        Kind("sms", "SMS", listOf(Manifest.permission.RECEIVE_SMS, Manifest.permission.READ_SMS), restrictedFromSdk = 35),
        Kind("mms", "MMS", listOf(Manifest.permission.READ_SMS), restrictedFromSdk = 35),
        Kind("calls", "Calls", listOf(Manifest.permission.READ_CALL_LOG, Manifest.permission.READ_PHONE_STATE), restrictedFromSdk = Int.MAX_VALUE),
        Kind("contacts", "Contacts", listOf(Manifest.permission.READ_CONTACTS), restrictedFromSdk = Int.MAX_VALUE),
        Kind("rcs", "RCS", emptyList(), listener = true, restrictedFromSdk = 33),
    )

    /** The capture kinds each capture setup step covers, by the step's title. */
    val STEP_KINDS = mapOf(
        "Messages" to listOf("sms", "mms"),
        "RCS messages" to listOf("rcs"),
        "Calls" to listOf("calls"),
        "Contacts" to listOf("contacts"),
    )

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** Background work [work] ran now: the catch-up worker, or the command fetch. */
    fun ran(context: Context, work: String, atMs: Long = System.currentTimeMillis()) {
        prefs(context).edit().putLong("ran.$work", atMs).apply()
    }

    /** When [work] last ran, or null when it never has. */
    fun lastRan(context: Context, work: String): Long? =
        prefs(context).let { if (it.contains("ran.$work")) it.getLong("ran.$work", 0) else null }

    /** Whether the command fetch runs on a 15-minute schedule, as it does with no push distributor. */
    fun setCommandsPolled(context: Context, polled: Boolean) {
        prefs(context).edit().putBoolean("commands_polled", polled).apply()
    }

    /** The user skipped the capture setup steps of [kinds]: they are not set up rather than missing. */
    fun skipped(context: Context, kinds: List<String>) {
        prefs(context).edit().apply { kinds.forEach { putBoolean("skipped.$it", true) } }.apply()
    }

    /** Android refused [kinds]' permissions without a dialog: restricted settings block them. */
    fun blocked(context: Context, kinds: List<String>) {
        prefs(context).edit().apply { kinds.forEach { putBoolean("blocked.$it", true) } }.apply()
    }

    /** Whether the app came from outside an app store, which Android's restricted settings watch. */
    fun sideloaded(context: Context): Boolean = try {
        context.packageManager.getInstallSourceInfo(context.packageName).installingPackageName !in STORES
    } catch (e: PackageManager.NameNotFoundException) {
        true
    }

    /**
     * A kind's state: [WORKING] once Android grants it; otherwise [NOT_SET_UP] before capture
     * setup is finished or when its step was skipped; [RESTRICTED] when restricted settings block
     * it, for an app from outside a store on a release that restricts it; else [PERMISSION_MISSING].
     */
    fun state(kind: Kind, granted: Boolean, setupDone: Boolean, skipped: Boolean, blocked: Boolean, sideloaded: Boolean, sdk: Int): String = when {
        granted -> WORKING
        !setupDone || skipped -> NOT_SET_UP
        sideloaded && sdk >= kind.restrictedFromSdk && (kind.listener || blocked) -> RESTRICTED
        else -> PERMISSION_MISSING
    }

    private fun granted(context: Context, kind: Kind): Boolean =
        kind.permissions.all { context.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED } &&
            (!kind.listener || RcsListenerService.enabled(context))

    private fun kindJson(kind: Kind, state: String): JSONObject {
        val o = JSONObject().put("kind", kind.id).put("label", kind.label).put("state", state)
        val missing = kind.permissions.joinToString(", ") { it.removePrefix("android.permission.") }
        when (state) {
            WORKING -> o.put("message", "Working").put("fix", JSONObject.NULL)
            NOT_SET_UP -> o.put("message", "Not set up").put("fix", FIX_CAPTURE_SETUP)
            PERMISSION_MISSING -> if (kind.listener) {
                o.put("message", "Notification access is off, so RCS messages are not captured")
                    .put("fix", FIX_NOTIFICATION_ACCESS)
            } else {
                o.put("message", "Permission missing: Android has not given TheVault $missing")
                    .put("fix", FIX_APP_DETAILS)
            }
            RESTRICTED -> {
                o.put(
                    "message",
                    "Blocked by restricted settings: in App info tap the three-dot menu, then Allow restricted settings, then " +
                        if (kind.listener) "turn on notification access" else "allow $missing",
                ).put("fix", FIX_APP_DETAILS)
                if (kind.listener) o.put("then", FIX_NOTIFICATION_ACCESS)
            }
        }
        return o
    }

    /** The problem line of a kind in [state], as the home server shows it, or null when it is none. */
    private fun problemLine(kind: Kind, state: String): String? = when (state) {
        PERMISSION_MISSING -> if (kind.listener) "${kind.label}: notification access is off" else "${kind.label}: permission missing"
        RESTRICTED -> "${kind.label}: blocked by restricted settings"
        else -> null
    }

    /**
     * The phone's capture health now: `{ paired, setup_done, kinds: [{ kind, label, state,
     * message, fix, then? }], battery: { optimised, samsung, message, fix }, background: [{ work,
     * label, last, expected, stale }], problems, persistent, checked }`. `problems` lists what
     * stops capture in words, `persistent` the ids of the problems that raise the notification.
     */
    fun check(context: Context, now: Long = System.currentTimeMillis()): JSONObject {
        val app = context.applicationContext
        val p = prefs(app)
        val paired = Device.deviceId(app) != null
        val setupDone = Device.setupDone(app)
        val sideloaded = sideloaded(app)
        val problems = JSONArray()
        val persistent = JSONArray()
        val kinds = JSONArray()
        val edit = p.edit()
        for (kind in KINDS) {
            val state = state(
                kind,
                granted(app, kind),
                setupDone,
                p.getBoolean("skipped.${kind.id}", false),
                p.getBoolean("blocked.${kind.id}", false),
                sideloaded,
                Build.VERSION.SDK_INT,
            )
            // Granted after all: a skip or a refusal no longer applies.
            if (state == WORKING) edit.remove("skipped.${kind.id}").remove("blocked.${kind.id}")
            kinds.put(kindJson(kind, state))
            problemLine(kind, state)?.let {
                problems.put(it)
                persistent.put("${kind.id}.permission")
            }
        }

        val power = app.getSystemService(PowerManager::class.java)
        val optimised = power != null && !power.isIgnoringBatteryOptimizations(app.packageName)
        val samsung = Build.MANUFACTURER.equals("samsung", ignoreCase = true)
        val battery = JSONObject().put("optimised", optimised).put("samsung", samsung)
        if (optimised) {
            battery.put("message", "Battery optimisation is on: Android may stop capture and sync running in the background")
                .put("fix", FIX_BATTERY)
            problems.put("Battery optimisation is on")
            persistent.put("battery")
        } else {
            battery.put("message", "Not optimised: capture and sync may run in the background").put("fix", JSONObject.NULL)
        }
        if (samsung) battery.put("samsung_line", SAMSUNG_LINE)

        val background = JSONArray()
        val watched = listOf(
            Triple(CATCH_UP, "Catch-up of messages, calls and contacts", paired && setupDone),
            Triple(COMMANDS, "Device command check", paired && p.getBoolean("commands_polled", false)),
        )
        for ((work, label, expected) in watched) {
            val last = lastRan(app, work)
            // Watched from the first check that expects it, so work that never ran counts too.
            val since = when {
                !expected -> null
                p.contains("since.$work") -> p.getLong("since.$work", now)
                else -> now.also { edit.putLong("since.$work", it) }
            }
            if (!expected) edit.remove("since.$work")
            val stale = since != null && now - maxOf(last ?: 0L, since) > STALE_MS
            background.put(
                JSONObject()
                    .put("work", work)
                    .put("label", label)
                    .put("last", last ?: JSONObject.NULL)
                    .put("expected", expected)
                    .put("stale", stale),
            )
            if (stale) {
                val hours = (now - maxOf(last ?: 0L, since!!)) / (60 * 60 * 1000L)
                problems.put("$label has not run for $hours hours")
                persistent.put("background.$work")
            }
        }
        edit.apply()

        return JSONObject()
            .put("paired", paired)
            .put("setup_done", setupDone)
            .put("kinds", kinds)
            .put("battery", battery)
            .put("background", background)
            .put("problems", problems)
            .put("persistent", persistent)
            .put("checked", now)
    }

    /**
     * What the home server keeps of [health]: the states and the problems, without the times
     * that change on every check, so the core sends it only when something changed.
     */
    fun report(health: JSONObject): JSONObject {
        val kinds = JSONArray()
        val all = health.getJSONArray("kinds")
        for (i in 0 until all.length()) {
            val k = all.getJSONObject(i)
            kinds.put(JSONObject().put("kind", k.getString("kind")).put("label", k.getString("label")).put("state", k.getString("state")))
        }
        val background = JSONArray()
        val works = health.getJSONArray("background")
        for (i in 0 until works.length()) {
            val w = works.getJSONObject(i)
            background.put(JSONObject().put("work", w.getString("work")).put("stale", w.getBoolean("stale")))
        }
        return JSONObject()
            .put("problems", health.getJSONArray("problems"))
            .put("kinds", kinds)
            .put("battery_optimised", health.getJSONObject("battery").getBoolean("optimised"))
            .put("background", background)
    }

    /**
     * Check, hand the core the report for its next settings sync, and, when [notify], raise the
     * notification for a persistent problem that has not raised it within a day. Answers the
     * health.
     */
    fun refresh(context: Context, notify: Boolean = true, now: Long = System.currentTimeMillis()): JSONObject {
        val health = check(context, now)
        try {
            VaultCore.setCaptureHealth(report(health).toString())
        } catch (e: IllegalStateException) {
            // The vault is not open: the next check hands it over.
        } catch (e: LinkageError) {
            Log.w(TAG, "the core is not loaded; capture health not reported", e)
        }
        if (notify) notifyDue(context, health, now)
        return health
    }

    private fun manager(context: Context) = context.getSystemService(NotificationManager::class.java)

    /** Raise the notification when a persistent problem in [health] has not raised it for a day. */
    fun notifyDue(context: Context, health: JSONObject, now: Long = System.currentTimeMillis()) {
        val p = prefs(context)
        val persistent = health.getJSONArray("persistent")
        val ids = (0 until persistent.length()).map { persistent.getString(it) }
        val due = ids.filter { now - p.getLong("notified.$it", Long.MIN_VALUE / 2) >= NOTIFY_EVERY_MS }
        if (due.isEmpty()) return
        val problems = health.getJSONArray("problems")
        val lines = (0 until problems.length()).map { problems.getString(it) }
        val text = lines.joinToString("\n") + "\nOpen TheVault to fix it."
        manager(context).createNotificationChannel(
            NotificationChannel(CHANNEL, "Capture health", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "A missing permission, battery optimisation or stalled background work that stops capture"
            },
        )
        val open = (
            context.packageManager.getLaunchIntentForPackage(context.packageName)
                ?: Intent(Intent.ACTION_MAIN).setPackage(context.packageName)
            )
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .putExtra(NotificationMirror.EXTRA_PATH, PATH)
        val notification = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_thevault)
            .setContentTitle(TITLE)
            .setContentText(lines.first())
            .setStyle(Notification.BigTextStyle().bigText(text))
            .setOnlyAlertOnce(true)
            .setAutoCancel(true)
            .setContentIntent(
                PendingIntent.getActivity(context, NOTIFICATION_ID, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT),
            )
            .build()
        try {
            manager(context).notify(NOTIFICATION_ID, notification)
        } catch (e: SecurityException) {
            // Notifications are not allowed: the panel still shows the problems.
            Log.w(TAG, "capture health not shown", e)
            return
        }
        p.edit().apply { due.forEach { putLong("notified.$it", now) } }.apply()
    }

    /** The system screen [fix] opens, or null for an unknown one. */
    fun fixIntent(context: Context, fix: String): Intent? {
        val pkg = Uri.fromParts("package", context.packageName, null)
        return when (fix) {
            FIX_APP_DETAILS -> Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, pkg)
            FIX_NOTIFICATION_ACCESS -> CaptureSetupActivity.listenerSettings(context)
            FIX_BATTERY -> Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, pkg)
            FIX_CAPTURE_SETUP -> Intent(context, CaptureSetupActivity::class.java)
            else -> null
        }
    }
}
