package dev.wiltaylor.thevault.device

import android.app.NotificationManager
import android.content.Context
import android.content.pm.ApplicationInfo
import androidx.test.core.app.ApplicationProvider
import androidx.work.Configuration
import androidx.work.ListenableWorker
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.testing.SynchronousExecutor
import androidx.work.testing.TestWorkerBuilder
import androidx.work.testing.WorkManagerTestInitHelper
import java.util.concurrent.Executors
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * The version gate (D27, ticket 112): before each round the phone compares the home server's
 * version with its own. Server newer and an APK for this phone published: the round neither
 * pushes nor pulls, and the update runs. Server newer and no APK: sync goes on with a warning.
 * Three failed updates: sync goes on with the warning. Runs the Rust core through JNI against a
 * fake home server on a free port.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class UpdateGateTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val homes = mutableListOf<FakeHome>()

    @Before
    fun setUp() {
        PairingStore.defaultCipher = FakeCipher
        WorkManagerTestInitHelper.initializeTestWorkManager(
            context,
            Configuration.Builder().setExecutor(SynchronousExecutor()).build(),
        )
        AppUpdateWorker.abisOverride = listOf("arm64-v8a")
        AppUpdateWorker.downloadsOverride = FakeDownloads()
        AppUpdateWorker.POLL_MS = 1
        installed("0.1.0")
        shadowOf(context.packageManager).setCanRequestPackageInstalls(true)
    }

    @After
    fun tearDown() {
        AppUpdateWorker.abisOverride = null
        AppUpdateWorker.downloadsOverride = null
        AppUpdateWorker.POLL_MS = 2_000L
        homes.forEach { it.close() }
    }

    private fun installed(version: String) {
        val info = shadowOf(context.packageManager).getInternalMutablePackageInfo(context.packageName)
        info.versionName = version
        info.longVersionCode = 1_000L
        context.applicationInfo.flags = context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE.inv()
        info.applicationInfo?.flags = context.applicationInfo.flags
    }

    private fun home(serverVersion: String): FakeHome {
        val home = FakeHome(serverVersion = serverVersion).also { homes.add(it) }
        PairingStore(context).save(
            JSONObject()
                .put("server_url", home.url)
                .put("device_id", FakeHome.DEVICE_ID)
                .put("device_name", "Pixel")
                .put("token", FakeHome.TOKEN),
        )
        // A note written on the phone waits in the queue.
        Vault.open(context)
        VaultCore.share("""{"text":"written before the update"}""")
        return home
    }

    private fun sync(): ListenableWorker.Result =
        TestWorkerBuilder.from(context, SyncWorker::class.java, Executors.newSingleThreadExecutor()).build().doWork()

    private fun update(): ListenableWorker.Result =
        TestWorkerBuilder.from(context, AppUpdateWorker::class.java, Executors.newSingleThreadExecutor()).build().doWork()

    private fun pending() = JSONObject(VaultCore.status()).getInt("pending")

    private fun gate() = JSONObject(VaultCore.status()).getJSONObject("update")

    private fun pulled(home: FakeHome) = home.requests.any { it.startsWith("GET /api/sync/changes") }

    private fun pushed(home: FakeHome) = home.requests.any { it.startsWith("PUT /api/notes/") }

    private fun gateWork() = WorkManager.getInstance(context).getWorkInfosForUniqueWork(UpdateGate.WORK).get()

    private fun notices() = shadowOf(context.getSystemService(NotificationManager::class.java)).allNotifications
        .filter { it.channelId == UpdateNotices.CHANNEL }

    @Test
    fun aNewerServerWithAMatchingBuildPausesSyncAndRunsTheUpdate() {
        val home = home(serverVersion = "0.2.0")
        home.publish("0.2.0", arch = "x86_64")
        home.publish("0.2.0")
        val result = sync()
        assertTrue("$result", result is ListenableWorker.Result.Success)
        assertEquals("0.2.0", result.outputData.getString(SyncWorker.PAUSED))
        assertTrue(home.requests.contains("GET /api/clients"))
        assertFalse("nothing is pulled", pulled(home))
        assertFalse("nothing is pushed", pushed(home))
        assertEquals(1, pending())
        assertEquals("pause", gate().getString("state"))
        assertEquals("Updating TheVault to 0.2.0 before syncing.", gate().getString("message"))
        val work = gateWork()
        assertEquals(1, work.size)
        assertEquals(WorkInfo.State.ENQUEUED, work.single().state)
        val notice = notices().single()
        assertEquals("Updating TheVault to 0.2.0 before syncing", notice.extras.getString("android.title"))

        // The new version starts: it syncs what waited.
        installed("0.2.0")
        val after = sync()
        assertTrue("$after", after is ListenableWorker.Result.Success)
        assertNotNull(after.outputData.getString(SyncWorker.REPORT))
        assertTrue(pulled(home))
        assertTrue(pushed(home))
        assertEquals(0, pending())
        assertEquals("sync", gate().getString("state"))
    }

    @Test
    fun aNewerServerWithNoMatchingBuildKeepsSyncingAndWarns() {
        val home = home(serverVersion = "0.2.0")
        // Only a build for another phone, and one older than the server.
        home.publish("0.2.0", arch = "x86_64")
        home.publish("0.1.5")
        val result = sync()
        assertTrue("$result", result is ListenableWorker.Result.Success)
        assertNull(result.outputData.getString(SyncWorker.PAUSED))
        assertNotNull(result.outputData.getString(SyncWorker.REPORT))
        assertTrue(pulled(home))
        assertTrue(pushed(home))
        assertEquals(0, pending())
        assertEquals("warn", gate().getString("state"))
        assertTrue(UpdateGate.warning()!!.contains("no matching app build"))
        assertTrue(gateWork().isEmpty())
    }

    @Test
    fun theSameVersionSyncsNormally() {
        val home = home(serverVersion = "0.1.0")
        home.publish("0.1.0")
        val result = sync()
        assertTrue("$result", result is ListenableWorker.Result.Success)
        assertNotNull(result.outputData.getString(SyncWorker.REPORT))
        assertFalse("the builds are not even read", home.requests.contains("GET /api/clients"))
        assertTrue(pulled(home))
        assertTrue(pushed(home))
        assertEquals("sync", gate().getString("state"))
        assertNull(UpdateGate.warning())
        assertTrue(gateWork().isEmpty())
    }

    @Test
    fun threeFailedUpdatesResumeSyncWithTheWarning() {
        val home = home(serverVersion = "0.2.0")
        // Every download comes out wrong.
        home.publish("0.2.0", sha256 = "0".repeat(64))
        for (n in 1..3) {
            val paused = sync()
            assertEquals("round $n", "0.2.0", paused.outputData.getString(SyncWorker.PAUSED))
            assertFalse(pulled(home))
            val failed = update()
            assertTrue("$failed", failed is ListenableWorker.Result.Failure)
            assertEquals(AppUpdateWorker.SHA_MISMATCH, failed.outputData.getString(AppUpdateWorker.STATUS))
            assertEquals(n, UpdateGate.failures(context, "0.2.0"))
        }
        // The later tries wait longer.
        assertEquals(0L, UpdateGate.backoffMs(0))
        assertTrue(UpdateGate.backoffMs(2) > UpdateGate.backoffMs(1))

        val result = sync()
        assertTrue("$result", result is ListenableWorker.Result.Success)
        assertNull(result.outputData.getString(SyncWorker.PAUSED))
        assertTrue(pulled(home))
        assertTrue(pushed(home))
        assertEquals(0, pending())
        val warning = UpdateGate.warning()!!
        assertTrue(warning, warning.contains("failed 3 times"))
        assertTrue(warning, warning.contains("SHA-256"))
    }
}
