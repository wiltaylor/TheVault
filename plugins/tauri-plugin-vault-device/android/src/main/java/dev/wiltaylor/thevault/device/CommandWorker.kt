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
 * Runs the commands server scripts queued for this phone (D22): the core takes the device's
 * settings from the home server, fetches the pending commands, runs each one the device file
 * declares and the settings enable, and reports every result. A `device.command` push starts it,
 * and so does the app opening; a phone with no UnifiedPush distributor also runs it every 15
 * minutes, beside the notification check (D4).
 *
 * What a command may do is enforced in the core and by Android, not here: `send_sms` needs the
 * command enabled, the device's SMS-send switch on, and the SEND_SMS permission.
 */
class CommandWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result {
        val paired = try {
            Device.openCore(applicationContext)
        } catch (e: IllegalStateException) {
            return Result.failure(workDataOf(ERROR to e.message))
        }
        if (!paired) return Result.success()
        return try {
            // A command's handler may write notes, as a sent SMS does: have them reach the server.
            if (JSONArray(VaultCore.runCommands()).length() > 0) SyncWorker.soon(applicationContext)
            Result.success()
        } catch (e: IllegalStateException) {
            if (runAttemptCount < MAX_ATTEMPTS) Result.retry() else Result.failure(workDataOf(ERROR to e.message))
        }
    }

    companion object {
        const val ERROR = "error"
        const val NOW = "thevault-commands-now"
        const val FALLBACK = "thevault-commands"

        private const val MAX_ATTEMPTS = 5

        /** Runs the pending commands now, after any run already waiting: for a push or the app opening. */
        fun enqueue(context: Context) {
            val request = OneTimeWorkRequestBuilder<CommandWorker>()
                .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork(NOW, ExistingWorkPolicy.APPEND_OR_REPLACE, request)
        }

        /** Starts the 15-minute check, for a phone that has no distributor, keeping one already set. */
        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<CommandWorker>(NotificationWorker.PERIOD_MINUTES, TimeUnit.MINUTES)
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
