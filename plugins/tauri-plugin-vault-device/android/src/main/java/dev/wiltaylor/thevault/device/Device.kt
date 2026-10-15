package dev.wiltaylor.thevault.device

import android.content.Context
import android.telephony.SubscriptionManager
import androidx.annotation.VisibleForTesting
import org.json.JSONObject
import java.util.concurrent.Executor
import java.util.concurrent.Executors

/**
 * The process's device capture, opened on first use by whatever starts first: the app, a
 * receiver, or a worker (D17). Tests replace [factory] and [executor].
 */
object Device {
    private const val PREFS = "thevault.device"
    private const val CURSORS = "thevault.device.cursors"
    private const val SETUP_DONE = "capture_setup_done"

    /** The device id the runtime opens with before pairing; events queue, but handlers wait. */
    const val UNPAIRED = "unpaired"

    @Volatile
    private var capture: DeviceCapture? = null

    /** Builds the capture for a context; tests swap in fakes. */
    @VisibleForTesting
    @Volatile
    var factory: (Context) -> DeviceCapture = ::production

    /** Where receivers and observers do their reading, off the main thread. */
    @VisibleForTesting
    @Volatile
    var executor: Executor = Executors.newSingleThreadExecutor()

    /** How long a receiver waits for the provider to hold the row it was told about, per try. */
    @VisibleForTesting
    @Volatile
    var retryDelayMs: Long = 1500

    fun capture(context: Context): DeviceCapture =
        capture ?: synchronized(this) { capture ?: factory(context.applicationContext).also { capture = it } }

    /** The device id the runtime was last opened with in this process. */
    @Volatile
    private var openedFor: String? = null

    @VisibleForTesting
    fun reset() {
        capture = null
        openedFor = null
    }

    /** The device id pairing gave this phone, or null before pairing. */
    fun deviceId(context: Context): String? = PairingStore(context.applicationContext).deviceId

    fun setupDone(context: Context): Boolean = prefs(context).getBoolean(SETUP_DONE, false)

    fun markSetupDone(context: Context) {
        prefs(context).edit().putBoolean(SETUP_DONE, true).apply()
    }

    /**
     * Open the vault with its pairing ([Vault.open]) and the device runtime with the paired
     * device id. Answers whether the device is paired.
     */
    fun openCore(context: Context): Boolean {
        val app = context.applicationContext
        val paired = Vault.open(app)
        val id = deviceId(app) ?: UNPAIRED
        VaultCore.openDevice(id, AndroidDeviceData(app, capture(app)))
        openedFor = id
        return paired
    }

    /** [openCore] unless this process already opened the runtime for the current device id. */
    fun ensureCore(context: Context) {
        if (openedFor != (deviceId(context) ?: UNPAIRED)) openCore(context)
    }

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private fun production(context: Context): DeviceCapture = DeviceCapture(
        context.contentResolver,
        PhoneNumbers.forDevice(context),
        CoreSink(context),
        Cursors(context.getSharedPreferences(CURSORS, Context.MODE_PRIVATE)),
        { filename, bytes ->
            ensureCore(context)
            JSONObject(VaultCore.putAsset(filename, bytes))
        },
        { selfNumbers(context, PhoneNumbers.forDevice(context)) },
    )

    /** This phone's own numbers, to leave out of a group MMS's recipients, when Android tells. */
    private fun selfNumbers(context: Context, numbers: PhoneNumbers): Set<String> = try {
        val subs = context.getSystemService(SubscriptionManager::class.java)
        subs.activeSubscriptionInfoList.orEmpty()
            .mapNotNull { numbers.normalise(subs.getPhoneNumber(it.subscriptionId)) }
            .toSet()
    } catch (_: SecurityException) {
        emptySet()
    }
}

/** Hands captured events to the device event queue in the Rust core, then has a worker run them. */
class CoreSink(private val context: Context) : EventSink {
    override fun enqueue(event: String, data: JSONObject, atMs: Long) {
        Device.ensureCore(context)
        VaultCore.enqueue(event, data.toString(), atMs)
    }

    override fun flush() = CaptureWork.runEvents(context)
}
