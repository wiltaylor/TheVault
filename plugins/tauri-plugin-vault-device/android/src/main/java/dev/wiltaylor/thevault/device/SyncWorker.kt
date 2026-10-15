package dev.wiltaylor.thevault.device

import android.content.Context
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
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
 * The output carries the round's report as JSON under [REPORT], or the error under [ERROR]. A
 * server that cannot be reached retries later; a server newer than the app fails the run and
 * keeps the queue until the app is updated (D27).
 */
class SyncWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result {
        val paired = try {
            Vault.open(applicationContext)
        } catch (e: IllegalStateException) {
            return Result.failure(workDataOf(ERROR to e.message))
        }
        if (!paired) return Result.success()
        return try {
            Result.success(workDataOf(REPORT to VaultCore.sync()))
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

        /** How often WorkManager syncs in the background: its shortest period. */
        const val PERIOD_MINUTES = 15L

        /** What the core's error says when the server is newer than the app (D27). */
        private const val UPDATE_THE_APP = "update the app"

        /** Schedules the periodic background sync, keeping a schedule that already exists. */
        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<SyncWorker>(PERIOD_MINUTES, TimeUnit.MINUTES)
                .setConstraints(
                    Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build(),
                )
                .build()
            WorkManager.getInstance(context)
                .enqueueUniquePeriodicWork(UNIQUE_NAME, ExistingPeriodicWorkPolicy.KEEP, request)
        }
    }
}
