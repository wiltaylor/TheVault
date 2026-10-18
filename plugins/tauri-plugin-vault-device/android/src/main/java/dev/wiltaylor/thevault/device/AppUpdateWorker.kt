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
import java.io.FileOutputStream
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
 * The download checks what it got against the list's `size` and the answer's `Content-Length`.
 * A short download is resumed with an HTTP `Range` request (`If-Range` names the SHA-256, so a
 * file replaced on the server starts over); one whose bytes are wrong starts over. After
 * [MAX_TRIES] tries it reports the SHA-256 and byte count it got.
 *
 * The version gate ([UpdateGate]) runs it too, when the home server is newer than the app and
 * sync waits for the update; every failure to update to a version counts against the gate.
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
        UpdateGate.installed(context, installed)
        val decision = AppUpdate.decide(ClientBuild.apks(manifest), abis(), installed, debuggable)
        val build = when (decision) {
            is UpdateDecision.Skip -> return done(decision.reason, decision.message)
            is UpdateDecision.Install -> decision.build
        }
        if (!context.packageManager.canRequestPackageInstalls()) {
            UpdateNotices.needsPermission(context, build.version)
            return done(NEEDS_PERMISSION, "Allow TheVault to install apps to update it to ${build.version}.", build.version)
        }
        val dir = File(context.cacheDir, "updates").apply { mkdirs() }
        dir.listFiles()?.forEach { it.delete() }
        val apk = File(dir, build.filename)
        when (val got = fetch("$server${build.path}", apk, build)) {
            is Fetched.Verified -> {}
            is Fetched.Wrong -> {
                apk.delete()
                val why = "the download did not match the server's: got ${got.size} bytes with SHA-256 ${got.sha256}, " +
                    "expected ${build.size} bytes with SHA-256 ${build.sha256}"
                UpdateNotices.failed(applicationContext, build.version, why)
                return done(SHA_MISMATCH, "After $MAX_TRIES tries ${why}; it was deleted.", build.version)
            }
            is Fetched.Failed -> {
                apk.delete()
                val why = "the download failed after $MAX_TRIES tries with ${got.size} bytes: ${got.error.message}"
                UpdateNotices.failed(applicationContext, build.version, why)
                return done(FAILED, "The download failed: ${got.error.message} (${got.size} bytes received).", build.version)
            }
        }
        archiveProblem(context, apk, build)?.let {
            apk.delete()
            UpdateNotices.failed(applicationContext, build.version, it)
            return done(REFUSED, it, build.version)
        }
        return try {
            AppInstaller.install(context, apk, build.version)
            done(INSTALLING, "Installing TheVault ${build.version}.")
        } catch (e: Exception) {
            Log.w(TAG, "installing ${build.filename}", e)
            UpdateNotices.failed(context, build.version, e.message ?: "the installer refused it")
            done(FAILED, "The installer refused it: ${e.message}", build.version)
        } finally {
            apk.delete()
        }
    }

    /**
     * Records the outcome. A failure to install [version] counts against the version gate
     * ([UpdateGate.failed]), which pauses sync for at most [MAX_TRIES] of them.
     */
    private fun done(status: String, message: String, version: String? = null): Result {
        applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putString(STATUS, status)
            .putString(MESSAGE, message)
            .putLong(CHECKED, System.currentTimeMillis())
            .apply()
        val failed = status == FAILED || status == SHA_MISMATCH || status == REFUSED
        if (version != null && (failed || status == NEEDS_PERMISSION)) {
            UpdateGate.failed(applicationContext, version, message)
        }
        val data = workDataOf(STATUS to status, MESSAGE to message)
        return if (failed) Result.failure(data) else Result.success(data)
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

        /** How many times one check downloads the APK, resuming or starting over, before it gives up. */
        const val MAX_TRIES = 3

        /** The ABIs the phone runs, preferred first; tests replace them. */
        @VisibleForTesting
        @Volatile
        var abisOverride: List<String>? = null

        internal fun abis(): List<String> = abisOverride ?: Build.SUPPORTED_ABIS.toList()

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

        /** What [fetch] got. */
        sealed class Fetched {
            /** The file has the list's size and SHA-256. */
            object Verified : Fetched()

            /** Every try ended with the wrong bytes; the last had [size] bytes and [sha256]. */
            data class Wrong(val sha256: String, val size: Long) : Fetched()

            /** The last try failed with [error], with [size] bytes on disk. */
            data class Failed(val error: Exception, val size: Long) : Fetched()
        }

        /**
         * Downloads [build] from [url] into [to], up to [MAX_TRIES] times: a short download is
         * resumed where it stopped, and one whose bytes are wrong starts over.
         */
        @VisibleForTesting
        fun fetch(url: String, to: File, build: ClientBuild): Fetched {
            to.delete()
            var wrong: Fetched.Wrong? = null
            var error: Exception? = null
            for (attempt in 1..MAX_TRIES) {
                try {
                    resume(url, to, build)
                } catch (e: IOException) {
                    // What arrived stays on disk for the next try to resume.
                    Log.w(TAG, "downloading ${build.filename}, try $attempt", e)
                    error = e
                    continue
                }
                // The server sent all it meant to: anything but the listed bytes is wrong.
                val size = to.length()
                val sha = sha256(to)
                if (sha == build.sha256 && (build.size < 0 || size == build.size)) return Fetched.Verified
                wrong = Fetched.Wrong(sha, size)
                error = null
                to.delete()
            }
            if (error == null && wrong != null) return wrong
            val size = if (to.exists()) to.length() else 0L
            return Fetched.Failed(error ?: IOException("the download failed"), size)
        }

        /**
         * One try: asks for the bytes after those [to] holds, or for the whole file, and appends
         * what arrives. Throws when the answer is shorter than its `Content-Length`.
         */
        private fun resume(url: String, to: File, build: ClientBuild) {
            val have = if (to.exists()) to.length() else 0L
            val conn = URL(url).openConnection() as HttpURLConnection
            conn.connectTimeout = 15_000
            conn.readTimeout = 60_000
            if (have > 0) {
                conn.setRequestProperty("Range", "bytes=$have-")
                // The server's ETag is the SHA-256: a different file is sent whole.
                conn.setRequestProperty("If-Range", "\"${build.sha256}\"")
            }
            try {
                val append = when (val code = conn.responseCode) {
                    206 -> true
                    200 -> false
                    416 -> if (build.size in 0..have) return else throw IOException("$url answered 416 at byte $have")
                    else -> throw IOException("$url answered $code")
                }
                val expected = conn.getHeaderField("Content-Length")?.toLongOrNull() ?: -1L
                var got = 0L
                conn.inputStream.use { input ->
                    FileOutputStream(to, append).use { out ->
                        val buf = ByteArray(64 * 1024)
                        while (true) {
                            val n = input.read(buf)
                            if (n < 0) break
                            out.write(buf, 0, n)
                            got += n
                        }
                    }
                }
                if (expected >= 0 && got != expected) {
                    throw IOException("got $got of the $expected bytes the server sent")
                }
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
