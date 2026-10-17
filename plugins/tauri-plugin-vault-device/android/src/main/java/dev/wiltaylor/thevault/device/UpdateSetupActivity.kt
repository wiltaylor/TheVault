package dev.wiltaylor.thevault.device

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import androidx.activity.ComponentActivity

/**
 * Explains, once, why TheVault asks to install apps: it updates itself with the APK your server
 * hands out, and Android lets an app install another APK only with the "Install unknown apps"
 * permission. The button opens that setting for TheVault; coming back with it granted starts the
 * update check ([AppUpdateWorker]) and closes.
 */
class UpdateSetupActivity : ComponentActivity() {
    lateinit var allowButton: Button
    lateinit var message: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val pad = (16 * resources.displayMetrics.density).toInt()
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad * 2, pad, pad)
        }
        column.addView(TextView(this).apply {
            text = "Let TheVault update itself"
            textSize = 22f
        })
        column.addView(TextView(this).apply {
            text = "When your server has a newer TheVault, the app downloads it from the server, checks it, and installs it. " +
                "Android needs your permission once for that: turn on \"Allow from this source\" for TheVault on the next screen. " +
                "The first update asks you to confirm; later ones install by themselves."
        })
        allowButton = Button(this).apply { text = "Allow updates" }
        column.addView(allowButton)
        message = TextView(this)
        column.addView(message)
        setContentView(column)
        allowButton.setOnClickListener { startActivity(settingsIntent()) }
    }

    override fun onResume() {
        super.onResume()
        if (packageManager.canRequestPackageInstalls()) {
            message.text = "TheVault can install its updates."
            AppUpdateWorker.enqueue(applicationContext)
            finish()
        }
    }

    /** The "Install unknown apps" setting for this app. */
    fun settingsIntent(): Intent =
        Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:$packageName"))
}
