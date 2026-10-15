package dev.wiltaylor.thevault.device

import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.webkit.WebView
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/**
 * TheVault's device plugin (note 01). When the app opens it hands the Rust core the stored
 * pairing and asks for a sync round, or starts onboarding on a device that is not paired yet;
 * background sync runs from [SyncWorker]. Once paired, it opens the capture setup screen until
 * that has been finished, and starts device capture (D7), which then runs from receivers and
 * workers with no webview. The notification listener and UnifiedPush code lands here too.
 *
 * Commands for the webview: `open_capture_setup` and `capture_status`.
 */
@TauriPlugin
class VaultDevicePlugin(private val activity: Activity) : Plugin(activity) {
    /** Whether this process already showed the capture setup screen. */
    private var setupShown = false

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

    /** Opens the vault with the stored pairing and, when paired, starts a round and capture. */
    private fun openAndSync(): Boolean {
        val paired = try {
            Vault.open(activity.applicationContext)
        } catch (e: IllegalStateException) {
            return false
        }
        if (paired) {
            VaultCore.kick()
            SyncWorker.schedule(activity.applicationContext)
            startCapture()
        }
        return paired
    }

    /** Capture runs once set up; right after pairing, onboarding continues with its setup. */
    private fun startCapture() {
        if (Device.setupDone(activity)) {
            CaptureWork.start(activity)
        } else if (!setupShown) {
            setupShown = true
            activity.startActivity(Intent(activity, CaptureSetupActivity::class.java))
        }
    }

    /** Open the capture setup screen: each permission in turn, then the first-run import. */
    @Command
    fun openCaptureSetup(invoke: Invoke) {
        activity.startActivity(Intent(activity, CaptureSetupActivity::class.java))
        invoke.resolve()
    }

    /** `{ setupDone, deviceId, permissions: { <permission>: granted } }`. */
    @Command
    fun captureStatus(invoke: Invoke) {
        val permissions = JSObject()
        CaptureSetupActivity.STEPS.flatMap { it.permissions }.forEach {
            permissions.put(it, activity.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED)
        }
        val status = JSObject()
        status.put("setupDone", Device.setupDone(activity))
        status.put("deviceId", Device.deviceId(activity))
        status.put("permissions", permissions)
        invoke.resolve(status)
    }
}
