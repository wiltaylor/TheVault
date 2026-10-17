package dev.wiltaylor.thevault.device

import android.Manifest
import android.app.Activity
import android.app.DatePickerDialog
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView
import java.time.LocalDate
import java.time.ZoneId

/**
 * The device capture part of onboarding (D7, note 02): each permission in turn, with the Allow
 * restricted settings steps when Android 15 blocks SMS for a sideloaded app, then notification
 * access for RCS (D13), then the first-run import, off by default, over a date range the user
 * picks.
 */
class CaptureSetupActivity : Activity() {
    /**
     * One permission screen. [restricted] permissions need Allow restricted settings on Android 15.
     * A [listener] step is notification-listener access, which is granted in Settings rather than
     * by a permission dialog.
     */
    data class Step(
        val title: String,
        val why: String,
        val permissions: List<String>,
        val restricted: Boolean = false,
        val listener: Boolean = false,
    )

    private var step = 0
    private lateinit var title: TextView
    private lateinit var body: TextView
    private lateinit var guidance: TextView
    private lateinit var primary: Button
    private lateinit var appInfo: Button
    private lateinit var skip: Button
    private lateinit var importPage: LinearLayout
    private lateinit var permissionPage: LinearLayout

    /** Whether the import runs, and its range: off, and the last 30 days, until the user changes them. */
    lateinit var importSwitch: Switch
        private set
    var importFrom: LocalDate = LocalDate.now().minusDays(DEFAULT_IMPORT_DAYS)
        private set
    var importTo: LocalDate = LocalDate.now()
        private set

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        step = savedInstanceState?.getInt(STEP) ?: 0
        setContentView(layout())
        show()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putInt(STEP, step)
    }

    override fun onResume() {
        super.onResume()
        // Back from App info, where the user may have allowed restricted settings and the permission.
        if (step < STEPS.size && granted(STEPS[step])) next()
    }

    private fun granted(s: Step) =
        s.permissions.all { checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED } &&
            (!s.listener || RcsListenerService.enabled(this))

    private fun next() {
        step++
        show()
    }

    /** The current step's screen, skipping steps already granted. */
    private fun show() {
        while (step < STEPS.size && granted(STEPS[step])) step++
        if (step >= STEPS.size) {
            permissionPage.visibility = View.GONE
            importPage.visibility = View.VISIBLE
            return
        }
        val s = STEPS[step]
        title.text = s.title
        body.text = s.why
        if (s.listener) {
            // Android 13 and later grey out notification access for a sideloaded app until the
            // user allows restricted settings, so the steps show from the start.
            guidance.text = LISTENER_GUIDANCE
            guidance.visibility = View.VISIBLE
            appInfo.visibility = View.VISIBLE
            primary.text = "Open notification access"
            primary.setOnClickListener { startActivity(listenerSettings(this)) }
            return
        }
        guidance.visibility = View.GONE
        appInfo.visibility = View.GONE
        primary.text = "Allow"
        primary.setOnClickListener { requestPermissions(s.permissions.toTypedArray(), REQUEST) }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, results: IntArray) {
        if (requestCode != REQUEST || step >= STEPS.size) return
        val s = STEPS[step]
        if (granted(s)) return next()
        // Android 15 refuses SMS to a sideloaded app without showing a dialog until the user
        // allows restricted settings from App info.
        val blocked = s.permissions.none { shouldShowRequestPermissionRationale(it) }
        if (s.restricted && blocked && Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM) {
            CaptureHealth.blocked(this, CaptureHealth.STEP_KINDS[s.title].orEmpty())
            guidance.text = RESTRICTED_GUIDANCE
            appInfo.visibility = View.VISIBLE
        } else {
            guidance.text = "Not allowed. TheVault works without it, and you can allow it later in Android's Settings > Apps > TheVault > Permissions."
        }
        guidance.visibility = View.VISIBLE
        primary.text = "Try again"
    }

    private fun finishSetup() {
        val app = applicationContext
        if (importSwitch.isChecked) {
            val zone = ZoneId.systemDefault()
            val from = importFrom.atStartOfDay(zone).toInstant().toEpochMilli()
            val to = importTo.plusDays(1).atStartOfDay(zone).toInstant().toEpochMilli() - 1
            CaptureWork.start(app)
            CaptureWork.importHistory(app, from, to)
        } else {
            CaptureWork.start(app)
        }
        Device.markSetupDone(app)
        setResult(RESULT_OK)
        finish()
    }

    private fun layout(): View {
        val pad = (16 * resources.displayMetrics.density).toInt()
        fun text(size: Float = 16f) = TextView(this).apply { textSize = size; setPadding(0, pad / 2, 0, pad / 2) }
        fun button(label: String, onClick: () -> Unit) = Button(this).apply { text = label; setOnClickListener { onClick() } }

        title = text(22f)
        body = text()
        guidance = text().apply { visibility = View.GONE }
        primary = button("Allow") {}
        appInfo = button("Open App info") {
            startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", packageName, null)))
        }.apply { visibility = View.GONE }
        skip = button("Skip") {
            if (step < STEPS.size) CaptureHealth.skipped(this, CaptureHealth.STEP_KINDS[STEPS[step].title].orEmpty())
            next()
        }
        permissionPage = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            listOf(title, body, guidance, primary, appInfo, skip).forEach { addView(it) }
        }

        importSwitch = Switch(this).apply {
            text = "Import messages, calls and contacts from before today"
            isChecked = DEFAULT_IMPORT
        }
        val range = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        lateinit var fromButton: Button
        lateinit var toButton: Button
        fun pick(initial: LocalDate, set: (LocalDate) -> Unit) {
            DatePickerDialog(this, { _, y, m, d -> set(LocalDate.of(y, m + 1, d)) }, initial.year, initial.monthValue - 1, initial.dayOfMonth).show()
        }
        fromButton = button("From $importFrom") { pick(importFrom) { importFrom = it; fromButton.text = "From $it" } }
        toButton = button("To $importTo") { pick(importTo) { importTo = it; toButton.text = "To $it" } }
        range.addView(fromButton)
        range.addView(toButton)
        range.visibility = if (DEFAULT_IMPORT) View.VISIBLE else View.GONE
        importSwitch.setOnCheckedChangeListener { _, on -> range.visibility = if (on) View.VISIBLE else View.GONE }
        importPage = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
            addView(text(22f).apply { text = "Earlier messages" })
            addView(
                text().apply {
                    text = "From now on TheVault captures new messages, calls and contact changes. " +
                        "It can also import what is already on the phone from a range of dates."
                },
            )
            addView(importSwitch)
            addView(range)
            addView(button("Finish") { finishSetup() })
        }

        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad, pad, pad)
            addView(permissionPage)
            addView(importPage)
        }
        return ScrollView(this).apply { addView(column) }
    }

    companion object {
        private const val REQUEST = 6601
        private const val STEP = "step"

        /** The first-run import is off by default (D7). */
        const val DEFAULT_IMPORT = false
        const val DEFAULT_IMPORT_DAYS = 30L

        const val RESTRICTED_GUIDANCE =
            "Android blocks SMS access for apps installed from outside Google Play until you allow it:\n" +
                "1. Tap Open App info.\n" +
                "2. Tap the three-dot menu at the top right, then Allow restricted settings, and confirm.\n" +
                "3. Come back here and tap Try again."

        const val LISTENER_GUIDANCE =
            "Android greys out notification access for apps installed from outside Google Play until you allow it:\n" +
                "1. Tap Open App info.\n" +
                "2. Tap the three-dot menu at the top right, then Allow restricted settings, and confirm.\n" +
                "3. Come back here, tap Open notification access, and turn on TheVault."

        /** Android's page for this app's notification-listener access. */
        fun listenerSettings(context: Context): Intent =
            Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS)
                .putExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME, RcsListenerService.component(context).flattenToString())

        val STEPS = listOf(
            Step(
                "Messages",
                "TheVault reads SMS and MMS as they arrive, so device scripts can file them as notes.",
                listOf(Manifest.permission.RECEIVE_SMS, Manifest.permission.READ_SMS),
                restricted = true,
            ),
            Step(
                "RCS messages",
                "Google Messages keeps RCS messages to itself, so TheVault reads them from its notifications. " +
                    "Messages you read in an open chat, messages you send and attachments are not captured.",
                emptyList(),
                listener = true,
            ),
            Step(
                "Calls",
                "TheVault reads the call log when a call ends, so device scripts can file calls as notes.",
                listOf(Manifest.permission.READ_CALL_LOG, Manifest.permission.READ_PHONE_STATE, Manifest.permission.READ_PHONE_NUMBERS),
            ),
            Step(
                "Contacts",
                "TheVault keeps phone contacts and people notes in step, and names who a message or call is from.",
                listOf(Manifest.permission.READ_CONTACTS, Manifest.permission.WRITE_CONTACTS),
            ),
            Step(
                "Sending SMS",
                "Only needed if you turn on SMS sending for this phone in Settings > Devices. Skip it otherwise.",
                listOf(Manifest.permission.SEND_SMS),
                restricted = true,
            ),
            Step(
                "Notifications",
                "TheVault shows notifications from your vault and from device scripts.",
                listOf(Manifest.permission.POST_NOTIFICATIONS),
            ),
        )
    }
}
