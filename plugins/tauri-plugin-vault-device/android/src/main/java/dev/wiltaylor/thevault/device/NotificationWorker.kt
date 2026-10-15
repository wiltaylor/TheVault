package dev.wiltaylor.thevault.device

import android.content.Context
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
import org.json.JSONArray

/**
 * Reads the home server's unread notifications and reconciles the shade with them
 * ([NotificationMirror.reconcile]). It runs when a push announces a new notification or a clear,
 * when the app opens, and, on a phone with no UnifiedPush distributor, every 15 minutes as the
 * fallback check (D4).
 */
class NotificationWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result {
        val paired = try {
            Vault.open(applicationContext)
        } catch (e: IllegalStateException) {
            return Result.failure(workDataOf(ERROR to e.message))
        }
        if (!paired) return Result.success()
        val unread = try {
            JSONArray(VaultCore.unreadNotifications())
        } catch (e: IllegalStateException) {
            return if (runAttemptCount < MAX_ATTEMPTS) Result.retry() else Result.failure(workDataOf(ERROR to e.message))
        }
        NotificationMirror.reconcile(applicationContext, unread, inputData.getString(PUSHED))
        return Result.success()
    }

    companion object {
        /** The id a `notification.created` push named, if this run is for one. */
        const val PUSHED = "pushed"
        const val ERROR = "error"
        const val NOW = "thevault-notifications-now"
        const val FALLBACK = "thevault-notifications"

        /** How often the fallback checks: WorkManager's shortest period. */
        const val PERIOD_MINUTES = 15L

        private const val MAX_ATTEMPTS = 5

        /** Reconciles now, after any run already waiting, for a push or the app opening. */
        fun enqueue(context: Context, pushed: String? = null) {
            val request = OneTimeWorkRequestBuilder<NotificationWorker>()
                .setInputData(workDataOf(PUSHED to pushed))
                .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork(NOW, ExistingWorkPolicy.APPEND_OR_REPLACE, request)
        }

        /** Starts the 15-minute check, for a phone that has no distributor, keeping one already set. */
        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<NotificationWorker>(PERIOD_MINUTES, TimeUnit.MINUTES)
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .build()
            WorkManager.getInstance(context)
                .enqueueUniquePeriodicWork(FALLBACK, ExistingPeriodicWorkPolicy.KEEP, request)
        }

        /** Stops the 15-minute check once pushes reach the phone. */
        fun unschedule(context: Context) {
            WorkManager.getInstance(context).cancelUniqueWork(FALLBACK)
        }
    }
}
