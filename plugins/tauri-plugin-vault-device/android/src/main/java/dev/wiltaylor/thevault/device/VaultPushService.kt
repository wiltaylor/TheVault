package dev.wiltaylor.thevault.device

import android.app.Activity
import android.content.Context
import androidx.work.Constraints
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import org.json.JSONObject
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.UnifiedPush
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/**
 * Receives UnifiedPush messages (D4) through `org.unifiedpush.android:connector`, which holds
 * the WebPush keys and decrypts each message (RFC 8291). A new endpoint is registered with the
 * home server; losing it, or having no distributor, falls back to the 15-minute check.
 */
class VaultPushService : PushService() {
    override fun onNewEndpoint(endpoint: PushEndpoint, instance: String) {
        val keys = endpoint.pubKeySet
        if (keys == null) {
            // The server only sends encrypted messages; with no keys it cannot reach this phone.
            NotificationWorker.schedule(applicationContext)
            return
        }
        PushEndpointWorker.enqueue(applicationContext, endpoint.url, keys.pubKey, keys.auth)
    }

    override fun onMessage(message: PushMessage, instance: String) {
        if (message.decrypted) PushHandler.handle(applicationContext, message.content)
    }

    override fun onRegistrationFailed(reason: FailedReason, instance: String) {
        NotificationWorker.schedule(applicationContext)
    }

    override fun onUnregistered(instance: String) {
        PushEndpointWorker.enqueue(applicationContext, null, null, null)
        NotificationWorker.schedule(applicationContext)
    }

    override fun onTempUnavailable(instance: String) {
        NotificationWorker.schedule(applicationContext)
    }
}

/**
 * What a decrypted push does. The server sends `{"event", "id"}` and no more (D9): a new
 * notification is fetched and shown, a read or dismissed one is cancelled at once, a clear
 * reconciles with the server's list, a device command has the pending commands fetched and run
 * (D22), and a note changed on the server (`{"event": "note.changed", "ids": [...]}`) has a sync
 * round pull it soon ([SyncWorker.soon]), so a change made elsewhere reaches the phone, and a
 * linked contact, without waiting for the periodic sync. A change to the phone's settings in
 * Settings > Devices (`{"event": "device.settings"}`) asks for the same round, which takes the
 * settings first, so a switch turned on applies, and the contacts catch up, at once. A new app
 * build published on the server (`{"event": "clients.updated"}`) has the update check run
 * ([AppUpdateWorker]), which installs it when it is newer.
 */
object PushHandler {
    /** Handles one decrypted message and answers its event, or null when it is not one. */
    fun handle(context: Context, content: ByteArray): String? {
        val message = try {
            JSONObject(String(content, Charsets.UTF_8))
        } catch (e: Exception) {
            return null
        }
        val event = message.optString("event")
        val id = message.optString("id").takeIf { it.isNotEmpty() && !message.isNull("id") }
        when (event) {
            "notification.created" -> NotificationWorker.enqueue(context, id)
            "notification.read", "notification.dismissed" -> id?.let { NotificationMirror.cancel(context, it) }
            "notification.cleared" -> NotificationWorker.enqueue(context)
            "device.command" -> CommandWorker.enqueue(context)
            "note.changed", "device.settings" -> SyncWorker.soon(context)
            "clients.updated" -> AppUpdateWorker.enqueue(context)
            else -> return null
        }
        return event
    }
}

/** Registers the phone's push endpoint with the home server, or clears it. */
class PushEndpointWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result {
        val paired = try {
            Vault.open(applicationContext)
        } catch (e: IllegalStateException) {
            return Result.failure()
        }
        if (!paired) return Result.success()
        val endpoint = inputData.getString(ENDPOINT)
        return try {
            VaultCore.setPush(endpoint, inputData.getString(P256DH), inputData.getString(AUTH))
            if (endpoint != null) NotificationWorker.unschedule(applicationContext)
            Result.success()
        } catch (e: IllegalStateException) {
            Result.retry()
        }
    }

    companion object {
        const val ENDPOINT = "endpoint"
        const val P256DH = "p256dh"
        const val AUTH = "auth"
        const val UNIQUE_NAME = "thevault-push-endpoint"

        fun enqueue(context: Context, endpoint: String?, p256dh: String?, auth: String?) {
            val request = OneTimeWorkRequestBuilder<PushEndpointWorker>()
                .setInputData(workDataOf(ENDPOINT to endpoint, P256DH to p256dh, AUTH to auth))
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork(UNIQUE_NAME, ExistingWorkPolicy.REPLACE, request)
        }
    }
}

/** Starting push for a paired phone. */
object PushRegistration {
    /**
     * Registers with the saved distributor, else the user's default one; the endpoint arrives in
     * [VaultPushService.onNewEndpoint]. With no distributor installed, the 15-minute check runs.
     */
    fun start(activity: Activity) {
        val context = activity.applicationContext
        UnifiedPush.tryUseCurrentOrDefaultDistributor(activity) { ok ->
            if (ok) {
                UnifiedPush.register(context, messageForDistributor = "TheVault notifications")
            } else {
                NotificationWorker.schedule(context)
            }
        }
    }
}
