package dev.wiltaylor.thevault.device

import android.content.Context
import android.database.ContentObserver
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.ContactsContract
import android.provider.Telephony
import android.util.Log
import androidx.annotation.VisibleForTesting
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import org.json.JSONObject

/**
 * The background side of capture (D7, D17): WorkManager jobs that run with no activity and no
 * webview, and content observers while the process lives.
 */
object CaptureWork {
    private const val RUN = "vault-device-run"
    private const val CATCH_UP = "vault-device-catch-up"
    private const val PROVIDERS = "vault-device-providers"
    private const val IMPORT = "vault-device-import"

    /** How often the catch-up reads every provider since its cursor. */
    const val CATCH_UP_HOURS = 1L

    /** The Telephony provider's SMS and MMS, which observers and the content-URI job watch. */
    val TELEPHONY: List<Uri> = listOf(Telephony.Sms.CONTENT_URI, Telephony.Mms.CONTENT_URI, Telephony.MmsSms.CONTENT_URI)

    /** Start capture: cursors, the periodic catch-up, the content-URI job and the observers. */
    fun start(context: Context) {
        val app = context.applicationContext
        Device.executor.execute { Device.capture(app).begin() }
        val work = WorkManager.getInstance(app)
        work.enqueueUniquePeriodicWork(
            CATCH_UP,
            ExistingPeriodicWorkPolicy.KEEP,
            PeriodicWorkRequestBuilder<CatchUpWorker>(CATCH_UP_HOURS, TimeUnit.HOURS).build(),
        )
        watchProviders(app, ExistingWorkPolicy.KEEP)
        CaptureObservers.register(app)
    }

    /** Have a worker run the queued events' handlers, after any run in progress. */
    fun runEvents(context: Context) {
        WorkManager.getInstance(context.applicationContext).enqueueUniqueWork(
            RUN,
            ExistingWorkPolicy.APPEND_OR_REPLACE,
            OneTimeWorkRequestBuilder<RunEventsWorker>().build(),
        )
    }

    /** The first-run import of every kind over [fromMs]..[toMs], in a worker. */
    fun importHistory(context: Context, fromMs: Long, toMs: Long) =
        importHistory(context, DeviceCapture.HISTORY_KINDS, fromMs, toMs)

    /**
     * Import [kinds] of history from [fromMs] to [toMs], null for no bound, in a worker, as the
     * first-run import and "Import from phone" in Settings > Devices do (D7). An import already
     * waiting is replaced; [ImportState] says how it goes.
     */
    fun importHistory(context: Context, kinds: List<String>, fromMs: Long?, toMs: Long?) {
        val request = ImportWorker.request(kinds, fromMs, toMs)
        ImportState.queued(request)
        WorkManager.getInstance(context.applicationContext).enqueueUniqueWork(
            IMPORT,
            ExistingWorkPolicy.REPLACE,
            OneTimeWorkRequestBuilder<ImportWorker>()
                .setInputData(workDataOf(ImportWorker.REQUEST to request.toString()))
                .build(),
        )
    }

    /**
     * A job that runs when contacts or the Telephony provider change, even with the app stopped.
     * A content-URI trigger fires once, so the job queues itself again each time it runs.
     */
    fun watchProviders(context: Context, policy: ExistingWorkPolicy) {
        val constraints = Constraints.Builder().apply {
            addContentUriTrigger(ContactsContract.Contacts.CONTENT_URI, true)
            TELEPHONY.forEach { addContentUriTrigger(it, true) }
            setTriggerContentUpdateDelay(5, TimeUnit.SECONDS)
            setTriggerContentMaxDelay(1, TimeUnit.MINUTES)
        }.build()
        WorkManager.getInstance(context.applicationContext).enqueueUniqueWork(
            PROVIDERS,
            policy,
            OneTimeWorkRequestBuilder<ProviderChangeWorker>().setConstraints(constraints).build(),
        )
    }
}

/**
 * Runs the queued device events' handlers in the Rust core, then has the notes they wrote reach
 * the home server soon ([SyncWorker.soon]). An event that failed for good is announced on the
 * phone ([CaptureProblems]) and reported to the home server by that round. Before pairing the
 * events wait.
 */
class RunEventsWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result = try {
        val app = applicationContext
        if (Device.openCore(app) && Device.deviceId(app) != null) {
            VaultCore.runPending()
            val failed = CaptureProblems.announce(app)
            if (failed > 0 || JSONObject(VaultCore.status()).optInt("pending") > 0) SyncWorker.soon(app)
        }
        Result.success()
    } catch (e: Exception) {
        Log.w(TAG, "running device events failed", e)
        Result.retry()
    }
}

/** The periodic catch-up: every provider since its cursor (D7). */
class CatchUpWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result = try {
        Device.capture(applicationContext).catchUp()
        Result.success()
    } catch (e: Exception) {
        Log.w(TAG, "catch-up failed", e)
        Result.retry()
    }
}

/** Contacts or the Telephony provider changed: capture what is new, and watch again. */
class ProviderChangeWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result {
        try {
            val capture = Device.capture(applicationContext)
            val n = capture.captureContacts() + capture.captureSms() + capture.captureMms()
            if (n > 0) capture.flush()
        } catch (e: Exception) {
            Log.w(TAG, "capture after a provider change failed", e)
        }
        CaptureWork.watchProviders(applicationContext, ExistingWorkPolicy.APPEND_OR_REPLACE)
        return Result.success()
    }
}

/**
 * An import of history over the kinds and dates the user picked (D7): at first run, or from
 * Settings > Devices. The Rust core reads the providers through the device bridge and files each
 * message and call that has no note yet; [ImportState] holds how it goes, for the page to show.
 */
class ImportWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result {
        val app = applicationContext
        val request = inputData.getString(REQUEST) ?: return Result.failure()
        return try {
            Device.capture(app).begin()
            Device.openCore(app)
            ImportState.started(request)
            val report = VaultCore.importHistory(request) { ImportState.progress(it) }
            ImportState.finished(report)
            if (JSONObject(VaultCore.status()).optInt("pending") > 0) SyncWorker.soon(app)
            Result.success()
        } catch (e: Exception) {
            Log.w(TAG, "import failed", e)
            ImportState.failed(e.message ?: e.toString())
            Result.failure()
        }
    }

    companion object {
        const val REQUEST = "request"

        /** The core's import request, `{ kinds, since, until }`, times in milliseconds or null. */
        fun request(kinds: List<String>, fromMs: Long?, toMs: Long?): JSONObject = JSONObject()
            .put("kinds", org.json.JSONArray(kinds))
            .put("since", fromMs ?: JSONObject.NULL)
            .put("until", toMs ?: JSONObject.NULL)
    }
}

/**
 * How the last import this process started is going, for Settings > Devices to show: `{ running,
 * request, report, error }`, where `report` is the core's report as it last heard it.
 */
object ImportState {
    @Volatile
    private var state: JSONObject = JSONObject().put("running", false)

    /** Hears every update too; tests watch the progress through it. */
    @Volatile
    var listener: ((JSONObject) -> Unit)? = null

    fun current(): JSONObject = JSONObject(state.toString())

    @Synchronized
    private fun set(next: JSONObject) {
        state = next
        listener?.invoke(current())
    }

    fun queued(request: JSONObject) = set(JSONObject().put("running", true).put("request", request))

    fun started(request: String) = set(JSONObject().put("running", true).put("request", JSONObject(request)))

    fun progress(report: String) = set(current().put("running", true).put("report", JSONObject(report)))

    fun finished(report: String) = set(current().put("running", false).put("report", JSONObject(report)))

    fun failed(error: String) = set(current().put("running", false).put("error", error))

    @VisibleForTesting
    fun reset() {
        state = JSONObject().put("running", false)
        listener = null
    }
}

/**
 * Content observers on the Telephony provider and contacts, while the process lives: how an MMS
 * is noticed (the provider, not the WAP push), and the quickest path for contact edits.
 */
object CaptureObservers {
    private val registered = AtomicBoolean(false)

    fun register(context: Context) {
        if (!registered.compareAndSet(false, true)) return
        val app = context.applicationContext
        val resolver = app.contentResolver
        val telephony = Coalescing { capture ->
            capture.captureSms() + capture.captureMms()
        }
        val contacts = Coalescing { capture -> capture.captureContacts() }
        CaptureWork.TELEPHONY.forEach { resolver.registerContentObserver(it, true, telephony.observer(app)) }
        resolver.registerContentObserver(ContactsContract.Contacts.CONTENT_URI, true, contacts.observer(app))
    }

    @VisibleForTesting
    fun reset() {
        registered.set(false)
    }

    /** Runs [read] once for a burst of changes, on the capture executor. */
    private class Coalescing(private val read: (DeviceCapture) -> Int) {
        private val queued = AtomicBoolean(false)

        fun observer(context: Context) = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) {
                if (!queued.compareAndSet(false, true)) return
                Device.executor.execute {
                    queued.set(false)
                    try {
                        val capture = Device.capture(context)
                        if (read(capture) > 0) capture.flush()
                    } catch (e: Exception) {
                        Log.w(TAG, "capture after a provider change failed", e)
                    }
                }
            }
        }
    }
}

private const val TAG = "VaultDeviceCapture"
