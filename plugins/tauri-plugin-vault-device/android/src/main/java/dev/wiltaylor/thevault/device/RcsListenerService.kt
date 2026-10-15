package dev.wiltaylor.thevault.device

import android.content.ComponentName
import android.content.Context
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import androidx.core.app.NotificationManagerCompat

/**
 * Reads Google Messages' notifications for RCS (D13), once the user grants notification-listener
 * access in onboarding. Every other app's notifications are ignored on arrival.
 */
class RcsListenerService : NotificationListenerService() {
    override fun onNotificationPosted(sbn: StatusBarNotification) {
        if (sbn.packageName != GoogleMessages.PACKAGE) return
        val app = applicationContext
        Device.executor.execute {
            try {
                // Google Messages writes an SMS to the provider before it notifies; the wait covers
                // a slow write, so the SMS is matched rather than captured again as RCS.
                if (Device.retryDelayMs > 0) Thread.sleep(Device.retryDelayMs)
                Device.rcs(app).capture(sbn.packageName, sbn.key, sbn.notification)
            } catch (e: Exception) {
                Log.w("VaultRcsCapture", "capture from a notification failed", e)
            }
        }
    }

    companion object {
        fun component(context: Context) = ComponentName(context, RcsListenerService::class.java)

        /** Whether the user granted notification-listener access. */
        fun enabled(context: Context): Boolean =
            context.packageName in NotificationManagerCompat.getEnabledListenerPackages(context)
    }
}
