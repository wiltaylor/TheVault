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

    /** The first-run import of [fromMs]..[toMs], in a worker. */
    fun importHistory(context: Context, fromMs: Long, toMs: Long) {
        WorkManager.getInstance(context.applicationContext).enqueueUniqueWork(
            IMPORT,
            ExistingWorkPolicy.REPLACE,
            OneTimeWorkRequestBuilder<ImportWorker>()
                .setInputData(workDataOf(ImportWorker.FROM to fromMs, ImportWorker.TO to toMs))
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
 * the home server soon ([SyncWorker.soon]). Before pairing the events wait.
 */
class RunEventsWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result = try {
        val app = applicationContext
        if (Device.openCore(app) && Device.deviceId(app) != null) {
            VaultCore.runPending()
            if (JSONObject(VaultCore.status()).optInt("pending") > 0) SyncWorker.soon(app)
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

/** The first-run import over the date range the user picked (D7). */
class ImportWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result = try {
        Device.capture(applicationContext).importHistory(inputData.getLong(FROM, 0), inputData.getLong(TO, 0))
        Result.success()
    } catch (e: Exception) {
        Log.w(TAG, "import failed", e)
        Result.failure()
    }

    companion object {
        const val FROM = "from"
        const val TO = "to"
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
