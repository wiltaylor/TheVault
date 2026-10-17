package dev.wiltaylor.thevault.device

import android.content.Context
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import androidx.annotation.VisibleForTesting
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.TimeUnit
import org.json.JSONObject

/**
 * The app updates itself from the paired home server (D27). The server hands out the APK it was
 * given with `just publish-clients` at the public `GET /api/clients`; this worker reads that list,
 * and when the APK for this phone is newer than the installed app and the same kind of build
 * ([AppUpdate.decide]), it downloads it into the cache folder, checks its SHA-256 against the
 * list, and hands it to [AppInstaller], which installs it with a PackageInstaller session.
 *
 * It runs when the server pushes `clients.updated` ([PushHandler]), when the app opens, and once
 * a day ([start]), whenever the phone is online; it needs neither idle nor charging. Installing
 * needs the user's "Install unknown apps" permission for TheVault, given once: without it the
 * worker posts a notice that opens [UpdateSetupActivity] and stops.
 *
 * The output carries what it decided under [STATUS] and why under [MESSAGE]; the last of each is
 * kept in the `thevault-update` preferences too ([lastStatus]).
 */
class AppUpdateWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result {
        val context = applicationContext
        val server = PairingStore(context).serverUrl ?: return done(NOT_PAIRED, "The phone is not paired.")
        val manifest = try {
            JSONObject(String(get("$server/api/clients"), Charsets.UTF_8))
        } catch (e: Exception) {
            Log.w(TAG, "reading the server's builds", e)
            return if (runAttemptCount < MAX_ATTEMPTS) Result.retry() else done(FAILED, "Cannot reach the server: ${e.message}")
        }
        val (installed, debuggable) = installed(context)
        val decision = AppUpdate.decide(ClientBuild.apks(manifest), abis(), installed, debuggable)
        val build = when (decision) {
            is UpdateDecision.Skip -> return done(decision.reason, decision.message)
            is UpdateDecision.Install -> decision.build
        }
        if (!context.packageManager.canRequestPackageInstalls()) {
            UpdateNotices.needsPermission(context, build.version)
            return done(NEEDS_PERMISSION, "Allow TheVault to install apps to update it to ${build.version}.")
        }
        val dir = File(context.cacheDir, "updates").apply { mkdirs() }
        dir.listFiles()?.forEach { it.delete() }
        val apk = File(dir, build.filename)
        try {
            download("$server${build.path}", apk)
        } catch (e: Exception) {
            apk.delete()
            Log.w(TAG, "downloading ${build.filename}", e)
            return if (runAttemptCount < MAX_ATTEMPTS) Result.retry() else done(FAILED, "The download failed: ${e.message}")
        }
        val sha = sha256(apk)
        if (sha != build.sha256) {
            apk.delete()
            UpdateNotices.failed(applicationContext, build.version, "the download did not match the server's SHA-256")
            return done(SHA_MISMATCH, "The downloaded APK's SHA-256 is $sha, not the server's ${build.sha256}; it was deleted.")
        }
        archiveProblem(context, apk, build)?.let {
            apk.delete()
            UpdateNotices.failed(applicationContext, build.version, it)
            return done(REFUSED, it)
        }
        return try {
            AppInstaller.install(context, apk, build.version)
            done(INSTALLING, "Installing TheVault ${build.version}.")
        } catch (e: Exception) {
            Log.w(TAG, "installing ${build.filename}", e)
            UpdateNotices.failed(context, build.version, e.message ?: "the installer refused it")
            done(FAILED, "The installer refused it: ${e.message}")
        } finally {
            apk.delete()
        }
    }

    private fun done(status: String, message: String): Result {
        applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putString(STATUS, status)
            .putString(MESSAGE, message)
            .putLong(CHECKED, System.currentTimeMillis())
            .apply()
        val data = workDataOf(STATUS to status, MESSAGE to message)
        return if (status == FAILED || status == SHA_MISMATCH || status == REFUSED) Result.failure(data) else Result.success(data)
    }

    companion object {
        const val STATUS = "status"
        const val MESSAGE = "message"
        private const val CHECKED = "checked"

        const val NOT_PAIRED = "not_paired"
        const val NEEDS_PERMISSION = "needs_permission"
        const val SHA_MISMATCH = "sha_mismatch"
        const val REFUSED = "refused"
        const val INSTALLING = "installing"
        const val FAILED = "failed"

        const val NOW = "thevault-update-now"
        const val DAILY = "thevault-update-daily"
        const val PREFS = "thevault-update"
        private const val TAG = "VaultUpdate"
        private const val MAX_ATTEMPTS = 3

        /** The ABIs the phone runs, preferred first; tests replace them. */
        @VisibleForTesting
        @Volatile
        var abisOverride: List<String>? = null

        private fun abis(): List<String> = abisOverride ?: Build.SUPPORTED_ABIS.toList()

        /** The installed app's version name, and whether it is a debug build. */
        fun installed(context: Context): Pair<String, Boolean> {
            val info = context.packageManager.getPackageInfo(context.packageName, 0)
            val debuggable = (context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0
            return (info.versionName ?: "0.0.0") to debuggable
        }

        /** When the last check finished, in milliseconds since the epoch, or null before the first. */
        fun lastChecked(context: Context): Long? {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            return if (prefs.contains(CHECKED)) prefs.getLong(CHECKED, 0) else null
        }

        /** The last check's status and message, or null before the first. */
        fun lastStatus(context: Context): Pair<String, String>? {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val status = prefs.getString(STATUS, null) ?: return null
            return status to prefs.getString(MESSAGE, "").orEmpty()
        }

        private fun online() = Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()

        /** Checks now, once the phone is online, as a push or the app opening asks. */
        fun enqueue(context: Context) {
            val request = OneTimeWorkRequestBuilder<AppUpdateWorker>().setConstraints(online()).build()
            WorkManager.getInstance(context).enqueueUniqueWork(NOW, ExistingWorkPolicy.REPLACE, request)
        }

        /** The daily check, keeping one already scheduled. */
        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<AppUpdateWorker>(1, TimeUnit.DAYS).setConstraints(online()).build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(DAILY, ExistingPeriodicWorkPolicy.KEEP, request)
        }

        /** What the app does when it opens paired: check now, and daily from then on. */
        fun start(context: Context) {
            try {
                schedule(context)
                enqueue(context)
            } catch (e: IllegalStateException) {
                Log.w(TAG, "no update check scheduled", e)
            }
        }

        private fun open(url: String): HttpURLConnection {
            val conn = URL(url).openConnection() as HttpURLConnection
            conn.connectTimeout = 15_000
            conn.readTimeout = 60_000
            if (conn.responseCode != 200) {
                val code = conn.responseCode
                conn.disconnect()
                throw IOException("$url answered $code")
            }
            return conn
        }

        private fun get(url: String): ByteArray {
            val conn = open(url)
            try {
                return conn.inputStream.use { it.readBytes() }
            } finally {
                conn.disconnect()
            }
        }

        private fun download(url: String, to: File) {
            val conn = open(url)
            try {
                conn.inputStream.use { input -> to.outputStream().use { input.copyTo(it) } }
            } finally {
                conn.disconnect()
            }
        }

        fun sha256(file: File): String {
            val digest = MessageDigest.getInstance("SHA-256")
            file.inputStream().use { input ->
                val buf = ByteArray(64 * 1024)
                while (true) {
                    val n = input.read(buf)
                    if (n < 0) break
                    digest.update(buf, 0, n)
                }
            }
            return digest.digest().joinToString("") { "%02x".format(it) }
        }

        /**
         * Why the APK must not be installed, when Android can read it: another package, or a
         * version code no newer than the installed one. When it cannot be read the installer
         * judges it, and refuses one that is not an APK.
         */
        private fun archiveProblem(context: Context, apk: File, build: ClientBuild): String? {
            val pm = context.packageManager
            val archive = pm.getPackageArchiveInfo(apk.path, PackageManager.PackageInfoFlags.of(0)) ?: return null
            if (archive.packageName != context.packageName) {
                return "The server's APK is ${archive.packageName}, not ${context.packageName}."
            }
            val mine = pm.getPackageInfo(context.packageName, 0).longVersionCode
            if (archive.longVersionCode <= mine) {
                return "The server's APK ${build.version} has version code ${archive.longVersionCode}, " +
                    "not newer than the installed $mine; TheVault never downgrades."
            }
            return null
        }
    }
}
