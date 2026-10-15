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
 * workers with no webview. A paired app also starts UnifiedPush once per process, and each time
 * it comes to the front reconciles the mirrored notifications and runs pending device commands; a
 * tapped notification opens its note here (D9, D22). The notification listener code lands here too.
 *
 * Commands for the webview: `open_capture_setup` and `capture_status`.
 */
@TauriPlugin
class VaultDevicePlugin(private val activity: Activity) : Plugin(activity) {
    /** Whether this process already showed the capture setup screen. */
    private var setupShown = false
    private var webView: WebView? = null

    override fun load(webView: WebView) {
        super.load(webView)
        this.webView = webView
        if (!openAndSync()) {
            activity.startActivity(Intent(activity, OnboardingActivity::class.java))
        }
        opened(activity.intent)
    }

    override fun onResume() {
        super.onResume()
        openAndSync()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        opened(intent)
    }

    /** Opens the vault with the stored pairing and, when paired, starts a round and capture. */
    private fun openAndSync(): Boolean {
        val context = activity.applicationContext
        val paired = try {
            Vault.open(context)
        } catch (e: IllegalStateException) {
            return false
        }
        if (paired) {
            VaultCore.kick()
            SyncWorker.schedule(context)
            startCapture()
            NotificationWorker.enqueue(context)
            CommandWorker.enqueue(context)
            if (!pushStarted) {
                pushStarted = true
                PushRegistration.start(activity)
            }
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
        status.put("notificationAccess", RcsListenerService.enabled(activity))
        invoke.resolve(status)
    }

    /** A tapped notification: mark it read and show its note or page. */
    private fun opened(intent: Intent?) {
        val id = intent?.getStringExtra(NotificationMirror.EXTRA_ID) ?: return
        val path = intent.getStringExtra(NotificationMirror.EXTRA_PATH)
        intent.removeExtra(NotificationMirror.EXTRA_ID)
        intent.removeExtra(NotificationMirror.EXTRA_PATH)
        MarkReadWorker.enqueue(activity.applicationContext, id)
        if (path != null) webView?.let { view -> view.post { view.loadUrl(ORIGIN + path) } }
    }

    companion object {
        /** Where the webview loads the app from (`apps/android`'s custom scheme). */
        const val ORIGIN = "http://vault.localhost"

        /** UnifiedPush registers once per process; the endpoint survives the app closing. */
        private var pushStarted = false
    }
}
