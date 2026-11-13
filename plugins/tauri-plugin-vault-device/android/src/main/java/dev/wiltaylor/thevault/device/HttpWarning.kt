package dev.wiltaylor.thevault.device

import android.app.Activity
import android.app.AlertDialog

/** Existing HTTP pairings remain usable, but do not authenticate the home feed. */
object HttpWarning {
    const val MESSAGE = "The home feed is not authenticated over plain HTTP. Someone on the network can read or change it. Use HTTPS when possible."

    fun show(activity: Activity, serverUrl: String?): Boolean {
        if (!serverUrl.orEmpty().trim().startsWith("http://", ignoreCase = true)) return false
        AlertDialog.Builder(activity)
            .setTitle("Unencrypted home connection")
            .setMessage(MESSAGE)
            .setPositiveButton("OK", null)
            .show()
        return true
    }
}
