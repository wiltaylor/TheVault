package dev.wiltaylor.thevault.device

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.telephony.SmsManager
import org.json.JSONArray
import org.json.JSONObject

/**
 * The device module's view of the phone (D23): reads through the same readers capture uses, and
 * contact writes, SMS sending and notifications through Android. The core enforces the device's
 * switches before it gets here; Android enforces its permissions, and a missing one comes back as
 * an error the script receives.
 */
class AndroidDeviceData(private val context: Context, private val capture: DeviceCapture) : DeviceBridge {
    override fun call(method: String, args: String): String = try {
        val a = JSONObject(args)
        val ok: Any = when (method) {
            "sms_list" -> JSONArray(capture.sms.list(a.optJSONObject("filter")))
            "sms_get" -> capture.sms.get(a.getString("device_ref"))
            "calls_list" -> JSONArray(capture.calls.list(a.optJSONObject("filter")))
            "contacts_list" -> JSONArray(capture.contacts.list(a.optJSONObject("filter")))
            "contacts_get" -> capture.contacts.get(a.getString("contact_id"))
            "contacts_upsert" -> capture.contacts.upsert(a.getJSONObject("contact"))
            "contacts_delete" -> JSONObject().also { capture.contacts.delete(a.getString("contact_id")) }
            "sms_send" -> sendSms(a.getString("to"), a.getString("body"))
            "show_notification" -> JSONObject().also {
                showNotification(a.optString("title"), a.optString("body"), a.optString("note"))
            }
            else -> throw IllegalArgumentException("the device has no $method")
        }
        JSONObject().put("ok", ok).toString()
    } catch (e: SecurityException) {
        err("Android has not given TheVault the permission $method needs: ${e.message}")
    } catch (e: Exception) {
        err(e.message ?: e.toString())
    }

    private fun err(message: String) = JSONObject().put("err", message).toString()

    private fun sendSms(to: String, body: String): JSONObject {
        if (context.checkSelfPermission(Manifest.permission.SEND_SMS) != PackageManager.PERMISSION_GRANTED) {
            throw SecurityException("SEND_SMS")
        }
        val sms = context.getSystemService(SmsManager::class.java)
        val parts = sms.divideMessage(body)
        if (parts.size > 1) sms.sendMultipartTextMessage(to, null, parts, null, null) else sms.sendTextMessage(to, null, body, null, null)
        // Android writes the sent message to the provider after it goes, where capture reads it
        // as `sms_sent`; its row id is not known yet.
        return JSONObject().put("device_ref", JSONObject.NULL)
    }

    private fun showNotification(title: String, body: String, note: String) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Device scripts", NotificationManager.IMPORTANCE_DEFAULT),
        )
        val open = context.packageManager.getLaunchIntentForPackage(context.packageName)?.apply {
            if (note.isNotEmpty()) data = Uri.parse("thevault://note/$note")
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val builder = android.app.Notification.Builder(context, CHANNEL)
            .setSmallIcon(context.applicationInfo.icon.takeIf { it != 0 } ?: android.R.drawable.ic_dialog_info)
            .setContentTitle(title)
            .setAutoCancel(true)
        if (body.isNotEmpty()) builder.setContentText(body)
        open?.let {
            builder.setContentIntent(
                PendingIntent.getActivity(context, note.hashCode(), it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT),
            )
        }
        manager.notify("device-script", (title + note + System.nanoTime()).hashCode(), builder.build())
    }

    companion object {
        const val CHANNEL = "device-scripts"
    }
}
