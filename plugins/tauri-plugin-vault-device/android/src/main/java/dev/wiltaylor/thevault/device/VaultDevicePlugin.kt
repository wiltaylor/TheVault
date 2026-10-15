package dev.wiltaylor.thevault.device

import android.app.Activity
import android.content.Intent
import android.webkit.WebView
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Plugin

/**
 * TheVault's device plugin (note 01). When the app opens it hands the Rust core the stored
 * pairing and asks for a sync round, or starts onboarding on a device that is not paired yet;
 * background sync runs from [SyncWorker]. The SMS, call log, contacts, notification listener and
 * UnifiedPush code lands here too.
 */
@TauriPlugin
class VaultDevicePlugin(private val activity: Activity) : Plugin(activity) {
    override fun load(webView: WebView) {
        super.load(webView)
        if (!openAndSync()) {
            activity.startActivity(Intent(activity, OnboardingActivity::class.java))
        }
    }

    override fun onResume() {
        super.onResume()
        openAndSync()
    }

    /** Opens the vault with the stored pairing and, when paired, starts a round. */
    private fun openAndSync(): Boolean {
        val paired = try {
            Vault.open(activity.applicationContext)
        } catch (e: IllegalStateException) {
            return false
        }
        if (paired) {
            VaultCore.kick()
            SyncWorker.schedule(activity.applicationContext)
        }
        return paired
    }
}
