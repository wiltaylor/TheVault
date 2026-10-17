package dev.wiltaylor.thevault.device

import android.content.Context
import android.util.Log
import androidx.annotation.VisibleForTesting
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import java.util.concurrent.TimeUnit

/**
 * Background sync (D19, D20): WorkManager runs one round of the Rust sync engine, through JNI,
 * with no webview. The foreground loop in the core covers the app while it is open.
 *
 * Two schedules run it: the periodic one ([schedule]), and a prompt one ([soon]) that background
 * capture asks for once a device event's handler has written notes, so a captured SMS, call or
 * contact change reaches the home server in seconds rather than at the next period. A
 * `note.changed` push from the home server asks for the prompt round too, so a change made
 * elsewhere reaches the phone while the app is closed, and so does a `device.settings` push: each
 * round first takes the device's settings from the home server.
 *
 * The output carries the round's report as JSON under [REPORT], or the error under [ERROR]. A
 * server that cannot be reached retries later; a server newer than the app fails the run and
 * keeps the queue until the app is updated (D27).
 */
class SyncWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result {
        if (inputData.getBoolean(PROMPT, false)) {
            // A prompt round waits for a burst of captures to land, so one round pushes them all.
            val settle = inputData.getLong(SETTLE_MS, 0)
            if (settle > 0) Thread.sleep(settle)
            begun()
        }
        val paired = try {
            // With the device runtime open, the round takes the device's settings first and
            // queues the person changes it pulls for their phone contacts.
            Device.openCore(applicationContext)
        } catch (e: IllegalStateException) {
            return Result.failure(workDataOf(ERROR to e.message))
        }
        if (!paired) return Result.success()
        return try {
            val report = VaultCore.sync()
            // The round may have run device events, the person changes it pulled; it reported
            // the failures among them to the home server, and the phone says so too.
            CaptureProblems.announce(applicationContext)
            Result.success(workDataOf(REPORT to report))
        } catch (e: IllegalStateException) {
            val message = e.message ?: "sync failed"
            if (message.contains(UPDATE_THE_APP)) {
                Result.failure(workDataOf(ERROR to message))
            } else {
                Result.retry()
            }
        }
    }

    companion object {
        const val REPORT = "report"
        const val ERROR = "error"
        const val UNIQUE_NAME = "thevault-sync"

        /** The unique name of the prompt round [soon] asks for. */
        const val SOON = "thevault-sync-soon"

        /** The input key of how long a prompt round waits, in milliseconds, before it syncs. */
        const val SETTLE_MS = "settle_ms"

        /** The input key that marks a prompt round, as [soon] asks for. */
        private const val PROMPT = "prompt"

        /** How often WorkManager syncs in the background: its shortest period. */
        const val PERIOD_MINUTES = 15L

        /** What the core's error says when the server is newer than the app (D27). */
        private const val UPDATE_THE_APP = "update the app"

        private const val TAG = "VaultSync"

        /**
         * How long a prompt round waits for more writes before it syncs. An expedited request
         * cannot carry an initial delay, so the debounce runs inside the worker.
         */
        @VisibleForTesting
        @Volatile
        var settleMs: Long = 2_000

        /**
         * Whether the app's window is in front, so the core's foreground loop runs rounds.
         * [VaultDevicePlugin] keeps it.
         */
        @Volatile
        var foreground: Boolean = false

        /** Schedules the periodic background sync, keeping a schedule that already exists. */
        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<SyncWorker>(PERIOD_MINUTES, TimeUnit.MINUTES)
                .setConstraints(online())
                .build()
            WorkManager.getInstance(context)
                .enqueueUniquePeriodicWork(UNIQUE_NAME, ExistingPeriodicWorkPolicy.KEEP, request)
        }

        /**
         * Notes were written in the background (a device event's handler, a share, a device
         * command): have them reach the home server soon. Nothing happens while the device is
         * not paired. With the app in front, the core's foreground loop takes the round;
         * otherwise WorkManager runs one once the phone is online.
         *
         * The request is expedited, so app standby does not hold it back, and runs as ordinary
         * work when the app's expedited quota is spent. The app's minimum SDK is 33, where
         * expedited work runs as an expedited job and never asks the worker for foreground info.
         *
         * A burst coalesces: while a prompt round waits to start (for the network, or for its
         * settle time), another request adds nothing; once its round has begun, the next request
         * appends one more round after it, so writes made during a round are not left for the
         * periodic sync. Which is the case is kept in this process rather than asked of
         * WorkManager, whose queries must not block a worker. Answers whether a round was asked
         * for.
         */
        fun soon(context: Context): Boolean {
            val app = context.applicationContext
            if (Device.deviceId(app) == null) return false
            if (foreground) {
                try {
                    VaultCore.kick()
                    return true
                } catch (e: IllegalStateException) {
                    Log.w(TAG, "the foreground loop could not be kicked; scheduling a round", e)
                }
            }
            synchronized(this) {
                val now = System.currentTimeMillis()
                // A round asked for longer ago than a period is not trusted to still be waiting.
                if (requestedAt != 0L && now - requestedAt < TimeUnit.MINUTES.toMillis(PERIOD_MINUTES)) return true
                return try {
                    val request = OneTimeWorkRequestBuilder<SyncWorker>()
                        .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
                        .setConstraints(online())
                        .setInputData(workDataOf(SETTLE_MS to settleMs, PROMPT to true))
                        .build()
                    WorkManager.getInstance(app)
                        .enqueueUniqueWork(SOON, ExistingWorkPolicy.APPEND_OR_REPLACE, request)
                    requestedAt = now
                    true
                } catch (e: IllegalStateException) {
                    // WorkManager is not initialised; the periodic sync takes the notes later.
                    Log.w(TAG, "no prompt sync scheduled", e)
                    false
                }
            }
        }

        /** When the prompt round waiting to begin was asked for, or 0 when none waits. */
        private var requestedAt = 0L

        /** A prompt round begins: what is written from now on needs a round of its own. */
        private fun begun() {
            synchronized(this) { requestedAt = 0L }
        }

        @VisibleForTesting
        fun resetPrompt() = begun()

        private fun online() = Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()
    }
}
