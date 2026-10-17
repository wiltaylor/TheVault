package dev.wiltaylor.thevault.device

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
import android.util.Log
import androidx.annotation.VisibleForTesting
import java.io.File

/**
 * Installs a downloaded APK over the running app with a PackageInstaller session. On Android 12
 * (API 31) and later the session asks for no user action
 * ([PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED]), which the manifest's
 * `UPDATE_PACKAGES_WITHOUT_USER_ACTION` permission allows for an update of the app by itself:
 * updates install silently, the first one over an APK installed by hand too. When the system
 * still asks (another app owns the app's updates, say), [UpdateInstallReceiver] posts its
 * confirmation as a notice to tap.
 */
object AppInstaller {
    const val ACTION_STATUS = "dev.wiltaylor.thevault.UPDATE_STATUS"
    const val EXTRA_VERSION = "dev.wiltaylor.thevault.update_version"

    /** The parameters of the last session made, for the JVM tests. */
    @VisibleForTesting
    @Volatile
    var lastParams: PackageInstaller.SessionParams? = null

    /** The session parameters for an update of this app from an APK of [size] bytes. */
    fun params(context: Context, size: Long): PackageInstaller.SessionParams =
        PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            setSize(size)
            setInstallReason(android.content.pm.PackageManager.INSTALL_REASON_USER)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
            }
        }

    /** Writes [apk] into a new session and commits it. Answers the session id. */
    fun install(context: Context, apk: File, version: String): Int {
        val installer = context.packageManager.packageInstaller
        val params = params(context, apk.length())
        lastParams = params
        val id = installer.createSession(params)
        context.getSharedPreferences(AppUpdateWorker.PREFS, Context.MODE_PRIVATE).edit()
            .putString(PENDING, version)
            // Written now, not later: installing the update ends this process.
            .commit()
        try {
            installer.openSession(id).use { session ->
                session.openWrite("thevault.apk", 0, apk.length()).use { out ->
                    apk.inputStream().use { it.copyTo(out) }
                    session.fsync(out)
                }
                val status = Intent(context, UpdateInstallReceiver::class.java)
                    .setAction(ACTION_STATUS)
                    .putExtra(EXTRA_VERSION, version)
                // The installer fills in the status extras, so the intent must be mutable.
                val sender = PendingIntent.getBroadcast(
                    context,
                    id,
                    status,
                    PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                )
                session.commit(sender.intentSender)
            }
        } catch (e: Exception) {
            installer.abandonSession(id)
            context.getSharedPreferences(AppUpdateWorker.PREFS, Context.MODE_PRIVATE).edit().remove(PENDING).apply()
            throw e
        }
        return id
    }

    /** The version an install was started for, until the new app has started. */
    internal const val PENDING = "pending"
}

/** The PackageInstaller session's result: done, waiting for the user's confirmation, or failed. */
class UpdateInstallReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != AppInstaller.ACTION_STATUS) return
        val version = intent.getStringExtra(AppInstaller.EXTRA_VERSION) ?: ""
        val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
        when (status) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                @Suppress("DEPRECATION")
                val confirm = intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT) ?: return
                UpdateNotices.confirm(context, version, confirm)
            }
            PackageInstaller.STATUS_SUCCESS -> UpdateNotices.updated(context, version)
            else -> {
                val message = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE) ?: "status $status"
                Log.w("VaultUpdate", "installing $version failed: $message")
                context.getSharedPreferences(AppUpdateWorker.PREFS, Context.MODE_PRIVATE).edit()
                    .remove(AppInstaller.PENDING)
                    .putString(AppUpdateWorker.STATUS, AppUpdateWorker.FAILED)
                    .putString(AppUpdateWorker.MESSAGE, "The installer refused $version: $message")
                    .apply()
                UpdateNotices.failed(context, version, message)
            }
        }
    }
}

/**
 * The updated app's first moment: Android sends `MY_PACKAGE_REPLACED` once it has replaced the
 * app, whoever installed it. When the update was this app's own, it says so.
 */
class PackageReplacedReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_MY_PACKAGE_REPLACED) return
        val prefs = context.getSharedPreferences(AppUpdateWorker.PREFS, Context.MODE_PRIVATE)
        if (prefs.getString(AppInstaller.PENDING, null) == null) return
        val version = try {
            AppUpdateWorker.installed(context).first
        } catch (e: Exception) {
            prefs.getString(AppInstaller.PENDING, "") ?: ""
        }
        UpdateNotices.updated(context, version)
    }
}

/** The update's notices, on a channel of their own. */
object UpdateNotices {
    const val CHANNEL = "thevault-updates"

    /** One notice at a time: each replaces the last. */
    const val ID = 0x7e5a

    private fun manager(context: Context) = context.getSystemService(NotificationManager::class.java)

    private fun post(context: Context, title: String, text: String, tap: PendingIntent?) {
        val nm = manager(context)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL, "App updates", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "TheVault updating itself from your server"
            },
        )
        // The status bar icon, as the mirrored notifications use: the launcher icon is an
        // adaptive icon, which is no notification small icon.
        val icon = R.drawable.ic_stat_thevault
        val builder = Notification.Builder(context, CHANNEL)
            .setSmallIcon(icon)
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(Notification.BigTextStyle().bigText(text))
            .setAutoCancel(true)
        if (tap != null) builder.setContentIntent(tap)
        nm.notify(ID, builder.build())
    }

    /** The update installed. */
    fun updated(context: Context, version: String) {
        context.getSharedPreferences(AppUpdateWorker.PREFS, Context.MODE_PRIVATE).edit()
            .remove(AppInstaller.PENDING)
            .putString(AppUpdateWorker.STATUS, "updated")
            .putString(AppUpdateWorker.MESSAGE, "TheVault updated to $version.")
            .apply()
        val open = context.packageManager.getLaunchIntentForPackage(context.packageName)
            ?.let { PendingIntent.getActivity(context, ID, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT) }
        post(context, "TheVault updated", "TheVault updated to $version.", open)
    }

    /** The update failed with [message]. */
    fun failed(context: Context, version: String, message: String) {
        post(context, "TheVault update failed", "TheVault $version did not install: $message", null)
    }

    /** The system asks the user to confirm the install: a tap opens its confirmation. */
    fun confirm(context: Context, version: String, confirm: Intent) {
        confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val tap = PendingIntent.getActivity(context, ID, confirm, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        post(context, "Update TheVault to $version", "Tap to install the update. Later updates install by themselves.", tap)
    }

    /** Installing needs the "Install unknown apps" permission: a tap explains and asks for it. */
    fun needsPermission(context: Context, version: String) {
        val intent = Intent(context, UpdateSetupActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val tap = PendingIntent.getActivity(context, ID, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        post(context, "TheVault $version is ready", "Allow TheVault to install apps once, so it can update itself from your server.", tap)
    }
}
