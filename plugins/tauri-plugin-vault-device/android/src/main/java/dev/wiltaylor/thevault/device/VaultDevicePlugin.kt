package dev.wiltaylor.thevault.device

import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.webkit.WebView
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONObject

/**
 * TheVault's device plugin (note 01). When the app opens it hands the Rust core the stored
 * pairing and asks for a sync round, or starts onboarding on a device that is not paired yet;
 * background sync runs from [SyncWorker]. Once paired, it opens the capture setup screen until
 * that has been finished, and starts device capture (D7), which then runs from receivers and
 * workers with no webview. A paired app also starts UnifiedPush once per process, and each time
 * it comes to the front reconciles the mirrored notifications and runs pending device commands; a
 * tapped notification opens its note here (D9, D22). While in front, the core follows the home
 * server's live events, so a note changed elsewhere is pulled within seconds (D4). The notification listener code lands here too.
 * It also keeps the page clear of the system bars and the keyboard ([SystemBars], ticket 79).
 *
 * Commands for the webview: `open_capture_setup`, `capture_status`, and for Settings > Devices'
 * "This device" section `import_history`, `import_status` and `check_now`, and for Settings >
 * General's About section `update_status` and `check_for_updates`.
 */
@TauriPlugin
class VaultDevicePlugin(private val activity: Activity) : Plugin(activity) {
    /** Whether this process already showed the capture setup screen. */
    private var setupShown = false
    private var webView: WebView? = null

    override fun load(webView: WebView) {
        super.load(webView)
        this.webView = webView
        SystemBars(activity, webView).install()
        if (!openAndSync()) {
            activity.startActivity(Intent(activity, OnboardingActivity::class.java))
        }
        opened(activity.intent)
    }

    override fun onResume() {
        super.onResume()
        SyncWorker.foreground = true
        if (openAndSync()) live(true)
    }

    /**
     * Out of the front: background writes sync through WorkManager ([SyncWorker.soon]), and
     * server changes arrive as a `note.changed` push instead of on the live stream.
     */
    override fun onPause() {
        super.onPause()
        SyncWorker.foreground = false
        live(false)
    }

    /** In front, the core follows the home server's live events and pulls each change (D4). */
    private fun live(on: Boolean) {
        try {
            VaultCore.foreground(on)
        } catch (e: IllegalStateException) {
            // The vault is not open: there is nothing to follow.
        }
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
            AppUpdateWorker.start(context)
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

    /**
     * "Import from phone" in Settings > Devices (D7): `{ kinds, since, until }`, times RFC 3339 or
     * null. Answers `{ started, missing: [{ kind, permission }] }`: when Android has not granted
     * what a chosen kind needs, nothing starts, so the page can explain and offer the permission
     * step. RCS is refused: Google Messages keeps it to itself.
     */
    @Command
    fun importHistory(invoke: Invoke) {
        val args = invoke.getArgs()
        val asked = args.optJSONArray("kinds")
        val kinds = (0 until (asked?.length() ?: 0)).map { asked!!.getString(it) }
        if ("rcs" in kinds) return invoke.reject(RCS_NOT_IMPORTABLE)
        val unknown = kinds.filter { it !in DeviceCapture.HISTORY_KINDS }
        if (kinds.isEmpty() || unknown.isNotEmpty()) {
            return invoke.reject("choose what to import: ${DeviceCapture.HISTORY_KINDS.joinToString(", ")}")
        }
        val missing = JSArray()
        for (kind in kinds) {
            val permission = DeviceCapture.HISTORY_PERMISSIONS.getValue(kind)
            if (activity.checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED) {
                missing.put(JSObject().put("kind", kind).put("permission", permission))
            }
        }
        val answer = JSObject().put("missing", missing)
        if (missing.length() > 0) return invoke.resolve(answer.put("started", false))
        fun time(name: String): Long? = args.optString(name).takeIf { it.isNotEmpty() && it != "null" }?.let {
            java.time.OffsetDateTime.parse(it).toInstant().toEpochMilli()
        }
        CaptureWork.importHistory(activity, kinds, time("since"), time("until"))
        invoke.resolve(answer.put("started", true))
    }

    /** `{ running, request, report, error }`: how the last import is going ([ImportState]). */
    @Command
    fun importStatus(invoke: Invoke) {
        invoke.resolve(JSObject(ImportState.current().toString()))
    }

    /**
     * "Check for new messages and calls now": the hourly catch-up, run at once. Answers `{
     * captured, missing }`: how many new items it found, and the capture permissions not granted.
     */
    @Command
    fun checkNow(invoke: Invoke) {
        val app = activity.applicationContext
        val missing = JSArray()
        DeviceCapture.HISTORY_PERMISSIONS.values.distinct().forEach {
            if (app.checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED) missing.put(it)
        }
        Device.executor.execute {
            try {
                val captured = Device.capture(app).catchUp()
                invoke.resolve(JSObject().put("captured", captured).put("missing", missing))
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.toString())
            }
        }
    }

    /**
     * Settings > General's update status (D27): `{ status, message, checked }`, what the last
     * update check decided, why, and when in milliseconds since the epoch; null before the first.
     */
    @Command
    fun updateStatus(invoke: Invoke) {
        val app = activity.applicationContext
        val answer = JSObject()
        val last = AppUpdateWorker.lastStatus(app)
        answer.put("status", last?.first ?: JSONObject.NULL)
        answer.put("message", last?.second ?: JSONObject.NULL)
        answer.put("checked", AppUpdateWorker.lastChecked(app) ?: JSONObject.NULL)
        invoke.resolve(answer)
    }

    /** Settings > General's "Check for updates": run the update check now, once online. */
    @Command
    fun checkForUpdates(invoke: Invoke) {
        AppUpdateWorker.enqueue(activity.applicationContext)
        invoke.resolve()
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

        /** Why RCS is not offered for import: `crates/embedded` says the same. */
        const val RCS_NOT_IMPORTABLE =
            "RCS messages cannot be imported: Google Messages keeps them to itself, and TheVault only sees them as their notifications arrive"
    }
}
