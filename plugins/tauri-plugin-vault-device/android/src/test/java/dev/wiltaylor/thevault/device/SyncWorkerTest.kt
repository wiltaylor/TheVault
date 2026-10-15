package dev.wiltaylor.thevault.device

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.work.ListenableWorker
import androidx.work.testing.TestWorkerBuilder
import java.util.concurrent.Executors
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * S3 on the device: the background sync worker runs the Rust sync engine through JNI, with no
 * webview, against a home server on a free port.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class SyncWorkerTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val homes = mutableListOf<FakeHome>()

    @Before
    fun useTheFakeKeystore() {
        PairingStore.defaultCipher = FakeCipher
    }

    @After
    fun stopServers() {
        homes.forEach { it.close() }
    }

    private fun home(protocol: Int = 1) = FakeHome(protocol).also { homes.add(it) }

    private fun pairWith(url: String) {
        PairingStore(context).save(
            JSONObject()
                .put("server_url", url)
                .put("device_id", FakeHome.DEVICE_ID)
                .put("device_name", "Pixel")
                .put("token", FakeHome.TOKEN),
        )
    }

    private fun runWorker(): ListenableWorker.Result =
        TestWorkerBuilder.from(context, SyncWorker::class.java, Executors.newSingleThreadExecutor())
            .build()
            .doWork()

    @Test
    fun theWorkerRunsARoundOfTheSyncEngine() {
        val home = home()
        pairWith(home.url)
        val result = runWorker()
        assertTrue("$result", result is ListenableWorker.Result.Success)
        val report = JSONObject(result.outputData.getString(SyncWorker.REPORT)!!)
        assertEquals(0, report.getInt("pulled"))
        // The engine's round, in its order: handshake, bases, the feed, assets, bases again.
        assertEquals(
            listOf(
                "GET /api/sync/info",
                "PUT /api/devices/${FakeHome.DEVICE_ID}/bases",
                "GET /api/sync/changes?since=0&limit=500",
                "GET /api/assets",
                "PUT /api/devices/${FakeHome.DEVICE_ID}/bases",
            ),
            home.requests.toList(),
        )
    }

    @Test
    fun aServerNewerThanTheAppFailsTheRunAndKeepsTheQueue() {
        val home = home(protocol = 999)
        pairWith(home.url)
        Vault.open(context)
        VaultCore.share("""{"text":"waits for the update"}""")
        val result = runWorker()
        assertTrue("$result", result is ListenableWorker.Result.Failure)
        assertTrue(result.outputData.getString(SyncWorker.ERROR)!!.contains("update the app"))
        assertEquals(listOf("GET /api/sync/info"), home.requests.toList())
        assertEquals(1, JSONObject(VaultCore.status()).getInt("pending"))
    }

    @Test
    fun anUnreachableServerRetriesLater() {
        val home = home()
        val url = home.url
        home.close()
        pairWith(url)
        assertTrue(runWorker() is ListenableWorker.Result.Retry)
    }

    @Test
    fun anUnpairedDeviceHasNothingToSync() {
        context.getSharedPreferences(PairingStore.PREFS, Context.MODE_PRIVATE).edit().clear().commit()
        assertTrue(runWorker() is ListenableWorker.Result.Success)
    }
}
