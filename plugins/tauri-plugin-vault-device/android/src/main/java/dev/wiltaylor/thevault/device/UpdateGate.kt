package dev.wiltaylor.thevault.device

import android.content.Context
import android.util.Log
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import java.util.concurrent.TimeUnit
import org.json.JSONObject

/**
 * The phone's half of the version gate (D27): the app updates itself before it syncs with a home
 * server newer than itself, so the server can be upgraded first and every app catches up before
 * it reads anything new. The rule lives in the Rust core (`crates/sync` `gate`), which asks it
 * before every round, the foreground loop's and [SyncWorker]'s alike:
 *
 * - server newer, and an APK of this app's kind for this phone at least as new as the server
 *   published: the round neither pushes nor pulls. [SyncWorker] then runs the update now
 *   ([updateFirst]), says "Updating TheVault to X before syncing" in a notification and in This
 *   device, and the new version syncs once it starts;
 * - server newer and no such APK: sync goes on, and This device warns;
 * - an update that fails keeps sync paused and is tried again, later each time; after three
 *   failures for one version sync goes on with the warning.
 *
 * This object tells the core which app it is ([tell]) and keeps the failed updates across
 * restarts, in the `thevault-update` preferences.
 */
object UpdateGate {
    /** The unique name of the update the gate asks for. */
    const val WORK = "thevault-update-gate"

    private const val FAILED_VERSION = "gate_failed_version"
    private const val FAILED_COUNT = "gate_failed_count"
    private const val FAILED_MESSAGE = "gate_failed_message"
    private const val TAG = "VaultUpdateGate"

    /** The gate paused sync to update the app to [version]; [message] says so. */
    data class Pause(val version: String, val message: String)

    private fun prefs(context: Context) =
        context.applicationContext.getSharedPreferences(AppUpdateWorker.PREFS, Context.MODE_PRIVATE)

    /** The installed app as the core's gate reads it: `{version, build, arches, failed}`. */
    fun app(context: Context): JSONObject {
        val (version, debuggable) = AppUpdateWorker.installed(context)
        val arches = org.json.JSONArray()
        AppUpdateWorker.abis().mapNotNull(AppUpdate::archOf).distinct().forEach { arches.put(it) }
        val out = JSONObject()
            .put("version", version)
            .put("build", if (debuggable) "debug" else "release")
            .put("arches", arches)
        val prefs = prefs(context)
        prefs.getString(FAILED_VERSION, null)?.let {
            out.put(
                "failed",
                JSONObject()
                    .put("version", it)
                    .put("count", prefs.getInt(FAILED_COUNT, 0))
                    .put("message", prefs.getString(FAILED_MESSAGE, null) ?: JSONObject.NULL),
            )
        }
        return out
    }

    /** Hands the core [app], as every round needs it first. */
    fun tell(context: Context) {
        try {
            VaultCore.setApp(app(context).toString())
        } catch (e: Throwable) {
            // No core in this process (a JVM test of the updater alone): the next round tells it.
            Log.w(TAG, "the core was not told the app's version", e)
        }
    }

    /** The updates to [version] that failed so far. */
    fun failures(context: Context, version: String): Int {
        val prefs = prefs(context)
        return if (prefs.getString(FAILED_VERSION, null) == version) prefs.getInt(FAILED_COUNT, 0) else 0
    }

    /** An update to [version] failed with [message]. Answers how many have failed for it. */
    fun failed(context: Context, version: String, message: String): Int {
        val count = failures(context, version) + 1
        prefs(context).edit()
            .putString(FAILED_VERSION, version)
            .putInt(FAILED_COUNT, count)
            .putString(FAILED_MESSAGE, message)
            .commit()
        tell(context)
        return count
    }

    /** The app runs [installed]: failures to update to it, or to an older version, are history. */
    fun installed(context: Context, installed: String) {
        val failed = prefs(context).getString(FAILED_VERSION, null) ?: return
        if ((AppUpdate.compare(installed, failed) ?: -1) >= 0) reset(context)
    }

    /** The new version started: forget the failures. */
    fun reset(context: Context) {
        prefs(context).edit().remove(FAILED_VERSION).remove(FAILED_COUNT).remove(FAILED_MESSAGE).commit()
    }

    private fun verdict(): JSONObject? = try {
        JSONObject(VaultCore.status()).optJSONObject("update")
    } catch (e: IllegalStateException) {
        null
    }

    /** The pause the gate's last verdict asks for, or null when it lets sync run. */
    fun pause(): Pause? {
        val v = verdict() ?: return null
        if (v.optString("state") != "pause") return null
        return Pause(v.optString("version"), v.optString("message"))
    }

    /** The gate's warning, while sync goes on without the update. */
    fun warning(): String? {
        val v = verdict() ?: return null
        return if (v.optString("state") == "warn") v.optString("message") else null
    }

    /** How long before the next try after [failures] failures: none at first, then a minute, four, sixteen. */
    fun backoffMs(failures: Int): Long =
        if (failures <= 0) 0 else TimeUnit.MINUTES.toMillis(minOf(60L, Math.pow(4.0, (failures - 1).toDouble()).toLong()))

    /**
     * Sync is paused for [pause]: say so, and run the update, now the first time and later after
     * each failure. An update already waiting to run is kept.
     */
    fun updateFirst(context: Context, pause: Pause) {
        val app = context.applicationContext
        UpdateNotices.updating(app, pause.version, pause.message)
        val request = OneTimeWorkRequestBuilder<AppUpdateWorker>()
            .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
            .setInitialDelay(backoffMs(failures(app, pause.version)), TimeUnit.MILLISECONDS)
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 1, TimeUnit.MINUTES)
            .build()
        try {
            WorkManager.getInstance(app).enqueueUniqueWork(WORK, ExistingWorkPolicy.KEEP, request)
        } catch (e: IllegalStateException) {
            Log.w(TAG, "no update scheduled", e)
        }
    }
}
