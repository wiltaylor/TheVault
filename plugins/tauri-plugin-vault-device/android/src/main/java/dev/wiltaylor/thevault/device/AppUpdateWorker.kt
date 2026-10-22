package dev.wiltaylor.thevault.device

import android.app.DownloadManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.net.Uri
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
import java.util.concurrent.locks.ReentrantLock
import org.json.JSONObject

/**
 * The app updates itself from the paired home server (D27). The server hands out the APK it was
 * given with `just publish-clients` at the public `GET /api/clients`; this worker reads that list,
 * and when the APK for this phone is newer than the installed app and the same kind of build
 * ([AppUpdate.decide]), it has Android's download service ([DownloadManager]) fetch it, checks
 * its size and SHA-256 against the list, and hands it to [AppInstaller], which installs it with a
 * PackageInstaller session.
 *
 * The download service, not this worker, holds the connection (ticket 140): it resumes a
 * download the network cut, keeps going while the app's process is gone, and shows its progress
 * in a notification. The download's id is kept in the `thevault-update` preferences, so the next
 * run attaches to it rather than starting again: the worker waits up to [WAIT_MS] for it, saying
 * how far it got under [STATUS] [DOWNLOADING], then asks WorkManager to run it again, and
 * [UpdateDownloadReceiver] runs it as soon as the download ends. A download the service gives up
 * on, or whose bytes are wrong, starts over; after [MAX_TRIES] downloads it reports the SHA-256
 * and byte count it got.
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
        val decision = AppUpdate.decide(
            ClientBuild.apks(manifest),
            abis(),
            installed,
            debuggable,
            manifest.optString("server_version").ifEmpty { null },
        )
        val build = when (decision) {
            is UpdateDecision.Skip -> {
                forget(context)
                return done(decision.reason, decision.message)
            }
            is UpdateDecision.Install -> decision.build
        }
        if (!context.packageManager.canRequestPackageInstalls()) {
            UpdateNotices.needsPermission(context, build.version)
            return done(NEEDS_PERMISSION, "Allow TheVault to install apps to update it to ${build.version}.", build.version)
        }
        // Another run in this process (the gate's, a push's) is already updating: it finishes.
        if (!running.tryLock()) return Result.retry()
        try {
            return update(context, server, build)
        } finally {
            running.unlock()
        }
    }

    private fun update(context: Context, server: String, build: ClientBuild): Result {
        // Left by the app before ticket 140, which downloaded into the cache folder itself.
        File(context.cacheDir, "updates").listFiles()?.forEach { it.delete() }
        val dir = context.getExternalFilesDir(UPDATES)?.apply { mkdirs() }
            ?: return done(FAILED, "There is no storage for the download.", build.version)
        val apk = File(dir, build.filename)
        val downloads = downloads(context)
        val got = fetch(context, downloads, "$server${build.path}", apk, build, WAIT_MS) { stopped() }
        when (got) {
            is Fetched.Verified -> {}
            is Fetched.Waiting -> {
                progress(context, build.version, got.bytes, got.total)
                return Result.retry()
            }
            is Fetched.Wrong -> {
                val why = "the download did not match the server's: got ${got.size} bytes with SHA-256 ${got.sha256}, " +
                    "expected ${build.size} bytes with SHA-256 ${build.sha256}"
                UpdateNotices.failed(applicationContext, build.version, why)
                return done(SHA_MISMATCH, "After $MAX_TRIES tries ${why}; it was deleted.", build.version)
            }
            is Fetched.Failed -> {
                val why = "the download failed after $MAX_TRIES tries with ${got.size} bytes: ${got.error.message}"
                UpdateNotices.failed(applicationContext, build.version, why)
                return done(FAILED, "The download failed: ${got.error.message} (${got.size} bytes received).", build.version)
            }
        }
        try {
            archiveProblem(context, apk, build)?.let {
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
            }
        } finally {
            forget(context)
            apk.delete()
        }
    }

    private fun stopped() = isStopped

    /** Says how far the download of [version] got, while it runs on. */
    private fun progress(context: Context, version: String, bytes: Long, total: Long) {
        val message = progressMessage(version, bytes, total)
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putString(STATUS, DOWNLOADING)
            .putString(MESSAGE, message)
            .putLong(CHECKED, System.currentTimeMillis())
            .apply()
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

        /** The download service is still fetching the APK; [MESSAGE] says how far it got. */
        const val DOWNLOADING = "downloading"

        const val NOW = "thevault-update-now"
        const val DAILY = "thevault-update-daily"
        const val PREFS = "thevault-update"
        private const val TAG = "VaultUpdate"
        private const val MAX_ATTEMPTS = 3

        /** How many times one update downloads the APK, starting over, before it gives up. */
        const val MAX_TRIES = 3

        /** The folder of the app's external files the download service writes the APK into. */
        private const val UPDATES = "updates"

        /** The download's id, the SHA-256 it is for, and how many downloads of it were started. */
        private const val DOWNLOAD_ID = "download_id"
        private const val DOWNLOAD_SHA = "download_sha256"
        private const val DOWNLOAD_TRIES = "download_tries"

        /** How long one run waits for the download, inside WorkManager's ten minutes. */
        @VisibleForTesting
        @Volatile
        var WAIT_MS = TimeUnit.MINUTES.toMillis(8)

        /** How often a run asks the download service how the download stands. */
        @VisibleForTesting
        @Volatile
        var POLL_MS = 2_000L

        /** One update at a time in this process. */
        private val running = ReentrantLock()

        /** The download service; tests replace it. */
        @VisibleForTesting
        @Volatile
        var downloadsOverride: Downloads? = null

        internal fun downloads(context: Context): Downloads = downloadsOverride ?: SystemDownloads(context)

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

        /**
         * Checks now, once the phone is online, as a push or the app opening asks. [policy] is
         * [ExistingWorkPolicy.KEEP] when a check already waiting or running will do.
         */
        fun enqueue(context: Context, policy: ExistingWorkPolicy = ExistingWorkPolicy.REPLACE) {
            val request = OneTimeWorkRequestBuilder<AppUpdateWorker>().setConstraints(online()).build()
            WorkManager.getInstance(context).enqueueUniqueWork(NOW, policy, request)
        }

        /** The id of the download the update waits for, or null. */
        fun pendingDownload(context: Context): Long? {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            return if (prefs.contains(DOWNLOAD_ID)) prefs.getLong(DOWNLOAD_ID, -1) else null
        }

        /** Cancels the download the update waited for, if any, deleting what it wrote. */
        internal fun forget(context: Context) {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            pendingDownload(context)?.let {
                try {
                    downloads(context).remove(it)
                } catch (e: Exception) {
                    Log.w(TAG, "removing download $it", e)
                }
            }
            prefs.edit().remove(DOWNLOAD_ID).remove(DOWNLOAD_SHA).remove(DOWNLOAD_TRIES).commit()
        }

        /** What the update says while the download runs. */
        fun progressMessage(version: String, bytes: Long, total: Long): String {
            fun mb(n: Long) = "%.1f".format(java.util.Locale.ROOT, n / 1_000_000.0)
            val of = if (total > 0) "${mb(bytes)} of ${mb(total)} MB" else "${mb(bytes)} MB"
            return "Downloading TheVault $version: $of."
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

            /** The download runs on: [bytes] of [total] (-1 when unknown) so far. */
            data class Waiting(val bytes: Long, val total: Long) : Fetched()

            /** Every try ended with the wrong bytes; the last had [size] bytes and [sha256]. */
            data class Wrong(val sha256: String, val size: Long) : Fetched()

            /** The last try failed with [error], with [size] bytes on disk. */
            data class Failed(val error: Exception, val size: Long) : Fetched()
        }

        /**
         * Has [downloads] fetch [build] from [url] into [to], or attaches to the download an
         * earlier run started for it, and waits up to [waitMs] for it to end, or until [stop].
         * A download that fails, or whose bytes are wrong, starts over, until [MAX_TRIES] were
         * started. Every outcome but [Fetched.Verified] and [Fetched.Waiting] forgets the
         * download; [Fetched.Verified] leaves [to] for the installer, and the caller to
         * [forget] it after.
         */
        @VisibleForTesting
        fun fetch(
            context: Context,
            downloads: Downloads,
            url: String,
            to: File,
            build: ClientBuild,
            waitMs: Long,
            stop: () -> Boolean = { false },
        ): Fetched {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            var id = pendingDownload(context)
            if (id != null && prefs.getString(DOWNLOAD_SHA, null) != build.sha256) {
                // A download of another build: the server has a newer one now.
                forget(context)
                id = null
            }
            if (id != null && downloads.query(id) == null) {
                // The service no longer knows it (the user cancelled it): a try that failed.
                Log.w(TAG, "download $id of ${build.filename} is gone")
                id = null
            }
            fun start(tries: Int): Long {
                to.parentFile?.listFiles()?.forEach { it.delete() }
                val started = downloads.enqueue(url, to, "TheVault ${build.version}")
                prefs.edit()
                    .putLong(DOWNLOAD_ID, started)
                    .putString(DOWNLOAD_SHA, build.sha256)
                    .putInt(DOWNLOAD_TRIES, tries)
                    .commit()
                return started
            }
            if (id == null) {
                val tries = if (prefs.getString(DOWNLOAD_SHA, null) == build.sha256) prefs.getInt(DOWNLOAD_TRIES, 0) else 0
                if (tries >= MAX_TRIES) {
                    forget(context)
                    return Fetched.Failed(IOException("the download was cancelled"), 0)
                }
                id = start(tries + 1)
            }
            val deadline = System.currentTimeMillis() + waitMs
            while (true) {
                val tries = prefs.getInt(DOWNLOAD_TRIES, 1)
                val d = downloads.query(id!!)
                if (d != null && d.state == Download.State.RUNNING) {
                    if (System.currentTimeMillis() >= deadline || stop()) return Fetched.Waiting(d.bytes, d.total)
                    Thread.sleep(POLL_MS)
                    continue
                }
                val retry: Fetched = when (d?.state) {
                    Download.State.DONE -> {
                        val size = if (to.exists()) to.length() else 0L
                        val sha = if (to.exists()) sha256(to) else ""
                        if (sha == build.sha256 && (build.size < 0 || size == build.size)) return Fetched.Verified
                        Log.w(TAG, "download $id of ${build.filename}, try $tries: $size bytes with SHA-256 $sha")
                        Fetched.Wrong(sha, size)
                    }
                    else -> {
                        val why = d?.let { "the download service gave up (${reason(it.reason)})" } ?: "the download was cancelled"
                        Log.w(TAG, "download $id of ${build.filename}, try $tries: $why")
                        Fetched.Failed(IOException(why), d?.bytes ?: 0L)
                    }
                }
                downloads.remove(id)
                to.delete()
                if (tries >= MAX_TRIES) {
                    forget(context)
                    return retry
                }
                id = start(tries + 1)
            }
        }

        /** Why the download service gave up, from its `COLUMN_REASON`. */
        private fun reason(code: Int): String = when (code) {
            DownloadManager.ERROR_CANNOT_RESUME -> "it cannot resume"
            DownloadManager.ERROR_DEVICE_NOT_FOUND -> "no storage"
            DownloadManager.ERROR_FILE_ALREADY_EXISTS -> "the file already exists"
            DownloadManager.ERROR_FILE_ERROR -> "a storage error"
            DownloadManager.ERROR_HTTP_DATA_ERROR -> "the connection failed"
            DownloadManager.ERROR_INSUFFICIENT_SPACE -> "not enough space"
            DownloadManager.ERROR_TOO_MANY_REDIRECTS -> "too many redirects"
            DownloadManager.ERROR_UNHANDLED_HTTP_CODE -> "an unexpected answer"
            in 100..599 -> "the server answered $code"
            else -> "reason $code"
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

/** How a download stands, as [Downloads] reports it. */
data class Download(val state: State, val bytes: Long, val total: Long, val reason: Int = 0) {
    enum class State { RUNNING, DONE, FAILED }
}

/** The download service as the update uses it: [SystemDownloads] on the phone, a fake in tests. */
interface Downloads {
    /** Starts downloading [url] into [to], titled [title] in its notification; answers its id. */
    fun enqueue(url: String, to: File, title: String): Long

    /** How download [id] stands, or null when the service knows none by that id. */
    fun query(id: Long): Download?

    /** Cancels download [id] and deletes what it wrote. */
    fun remove(id: Long)
}

/**
 * Android's [DownloadManager]: it resumes a download the network cut, survives the app's process,
 * and shows its progress in a notification. It writes only into the app's external files.
 */
class SystemDownloads(private val context: Context) : Downloads {
    private val manager: DownloadManager
        get() = context.getSystemService(DownloadManager::class.java)

    override fun enqueue(url: String, to: File, title: String): Long {
        val request = DownloadManager.Request(Uri.parse(url))
            .setDestinationUri(Uri.fromFile(to))
            .setTitle(title)
            .setDescription("Updating TheVault from your server")
            .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE)
            .setAllowedOverMetered(true)
            .setAllowedOverRoaming(true)
        return manager.enqueue(request)
    }

    override fun query(id: Long): Download? {
        manager.query(DownloadManager.Query().setFilterById(id)).use { c ->
            if (c == null || !c.moveToFirst()) return null
            fun long(column: String): Long = c.getColumnIndex(column).takeIf { it >= 0 }?.let { c.getLong(it) } ?: -1L
            fun int(column: String): Int = c.getColumnIndex(column).takeIf { it >= 0 }?.let { c.getInt(it) } ?: -1
            val status = int(DownloadManager.COLUMN_STATUS)
            val state = when (status) {
                DownloadManager.STATUS_SUCCESSFUL -> Download.State.DONE
                DownloadManager.STATUS_FAILED -> Download.State.FAILED
                else -> Download.State.RUNNING
            }
            return Download(
                state,
                long(DownloadManager.COLUMN_BYTES_DOWNLOADED_SO_FAR).coerceAtLeast(0),
                long(DownloadManager.COLUMN_TOTAL_SIZE_BYTES),
                int(DownloadManager.COLUMN_REASON),
            )
        }
    }

    override fun remove(id: Long) {
        manager.remove(id)
    }
}

/**
 * The download service finished a download: when it is the update's, the update check runs now
 * to verify and install it, unless one is already waiting or running.
 */
class UpdateDownloadReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != DownloadManager.ACTION_DOWNLOAD_COMPLETE) return
        val id = intent.getLongExtra(DownloadManager.EXTRA_DOWNLOAD_ID, -1)
        if (id < 0 || id != AppUpdateWorker.pendingDownload(context)) return
        try {
            AppUpdateWorker.enqueue(context.applicationContext, ExistingWorkPolicy.KEEP)
        } catch (e: IllegalStateException) {
            Log.w("VaultUpdate", "no update check scheduled", e)
        }
    }
}
