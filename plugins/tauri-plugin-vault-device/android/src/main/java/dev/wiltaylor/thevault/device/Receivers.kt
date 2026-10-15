package dev.wiltaylor.thevault.device

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.provider.Telephony
import android.telephony.TelephonyManager
import android.util.Log

/**
 * Reads [read] on the capture executor, trying again while it captures nothing: the broadcast can
 * arrive before the default SMS app or the dialer has written the row.
 */
private fun BroadcastReceiver.captureSoon(context: Context, read: (DeviceCapture) -> Int) {
    val pending = goAsync()
    val capture = Device.capture(context)
    Device.executor.execute {
        try {
            var tries = 0
            while (read(capture) == 0 && ++tries < RECEIVER_TRIES) Thread.sleep(Device.retryDelayMs)
            if (tries < RECEIVER_TRIES) capture.flush()
        } catch (e: Exception) {
            Log.w("VaultDeviceCapture", "capture after a broadcast failed", e)
        } finally {
            pending?.finish()
        }
    }
}

/** Tries within a receiver's ten seconds; the catch-up gets anything later. */
private const val RECEIVER_TRIES = 5

/** `SMS_RECEIVED`: capture the new SMS from the provider, so it carries its row id (D7). */
class SmsReceivedReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Telephony.Sms.Intents.SMS_RECEIVED_ACTION) return
        captureSoon(context) { it.captureSms() }
    }
}

/** `PHONE_STATE` back to idle: a call ended, so re-read the call log (D7). */
class PhoneStateReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != TelephonyManager.ACTION_PHONE_STATE_CHANGED) return
        if (intent.getStringExtra(TelephonyManager.EXTRA_STATE) != TelephonyManager.EXTRA_STATE_IDLE) return
        captureSoon(context) { it.captureCalls() }
    }
}
