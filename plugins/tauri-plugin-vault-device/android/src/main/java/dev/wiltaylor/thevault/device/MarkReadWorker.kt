package dev.wiltaylor.thevault.device

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.work.Constraints
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import androidx.work.workDataOf

/**
 * Marks one notification read on the home server, for a swipe or a tap (D9). It waits for a
 * network and retries, so a swipe made offline still lands.
 */
class MarkReadWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result {
        val id = inputData.getString(ID) ?: return Result.failure()
        val paired = try {
            Vault.open(applicationContext)
        } catch (e: IllegalStateException) {
            return Result.failure()
        }
        if (!paired) return Result.success()
        return try {
            VaultCore.markRead(id)
            Result.success()
        } catch (e: IllegalStateException) {
            Result.retry()
        }
    }

    companion object {
        const val ID = "id"

        fun uniqueName(id: String) = "thevault-read-$id"

        fun enqueue(context: Context, id: String) {
            val request = OneTimeWorkRequestBuilder<MarkReadWorker>()
                .setInputData(workDataOf(ID to id))
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork(uniqueName(id), ExistingWorkPolicy.KEEP, request)
        }
    }
}

/** A notification swiped away: mark it read on the server (D9). */
class NotificationDismissReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != NotificationMirror.ACTION_DISMISSED) return
        val id = intent.getStringExtra(NotificationMirror.EXTRA_ID) ?: return
        MarkReadWorker.enqueue(context, id)
    }
}
