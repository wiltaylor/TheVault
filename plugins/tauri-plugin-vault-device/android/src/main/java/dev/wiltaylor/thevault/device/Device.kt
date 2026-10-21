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

    /** The asset each contact photo was stored as, by the SHA-256 of its bytes. */
    private const val PHOTOS = "thevault.device.contact_photos"
    private const val RCS = "thevault.device.rcs"
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

    @Volatile
    private var rcs: RcsCapture? = null

    /** Builds the RCS capture for a context; tests swap in fakes. */
    @VisibleForTesting
    @Volatile
    var rcsFactory: (Context) -> RcsCapture = { app ->
        RcsCapture(app.contentResolver, CoreSink(app), SeenRefs(app.getSharedPreferences(RCS, Context.MODE_PRIVATE)))
    }

    /** The process's RCS capture from Google Messages notifications (D13). */
    fun rcs(context: Context): RcsCapture =
        rcs ?: synchronized(this) { rcs ?: rcsFactory(context.applicationContext).also { rcs = it } }

    /** The device id the runtime was last opened with in this process. */
    @Volatile
    private var openedFor: String? = null

    @VisibleForTesting
    fun reset() {
        capture = null
        rcs = null
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
        UpdateGate.tell(app)
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

    /**
     * The local vault's assets, through the core. A captured part streams to a file in the app's
     * files folder, on the vault's filesystem, which the core moves into the vault (ticket 141).
     */
    fun coreAssets(context: Context): AssetStore = object : AssetStore {
        override fun put(filename: String, bytes: ByteArray): JSONObject {
            ensureCore(context)
            return JSONObject(VaultCore.putAsset(filename, bytes))
        }

        override fun bytes(asset: String): ByteArray? {
            ensureCore(context)
            return VaultCore.assetBytes(asset)
        }

        override fun putStream(filename: String, input: java.io.InputStream, cap: Long): JSONObject {
            ensureCore(context)
            val dir = java.io.File(context.applicationContext.filesDir, "incoming")
            return streamToFile(dir, input, cap) { file -> JSONObject(VaultCore.putAssetFile(file.path, filename)) }
        }
    }

    private fun production(context: Context): DeviceCapture = DeviceCapture(
        context.contentResolver,
        PhoneNumbers.forDevice(context),
        CoreSink(context),
        Cursors(context.getSharedPreferences(CURSORS, Context.MODE_PRIVATE)),
        coreAssets(context),
        { selfNumbers(context, PhoneNumbers.forDevice(context)) },
        PrefsMap(context.getSharedPreferences(PHOTOS, Context.MODE_PRIVATE), ""),
        vaultLinks(context),
    )

    /**
     * The contact links of the local vault's vault (ticket 143): its id from the core, and a
     * person it holds, by the note's file.
     */
    fun vaultLinks(context: Context): ContactLinks {
        val app = context.applicationContext
        return ContactLinks(
            vaultId = {
                try {
                    ensureCore(app)
                    VaultCore.vaultId()
                } catch (e: Exception) {
                    null
                }
            },
            personExists = { id -> java.io.File(Vault.dir(app), "notes/person/$id.wcl").isFile },
        )
    }

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
