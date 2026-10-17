package dev.wiltaylor.thevault.device

import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageInstaller
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import androidx.work.Configuration
import androidx.work.ListenableWorker
import androidx.work.NetworkType
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.testing.SynchronousExecutor
import androidx.work.testing.TestWorkerBuilder
import androidx.work.testing.WorkManagerTestInitHelper
import java.io.File
import java.security.MessageDigest
import java.util.Collections
import java.util.concurrent.Executors
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import okio.Buffer
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * S5, self-update (D27): a `clients.updated` push schedules the update check; the check reads the
 * paired server's `GET /api/clients`, downloads the newer APK for this phone, verifies its
 * SHA-256, and creates a PackageInstaller session that asks for no user action. It never
 * downgrades, refuses a download whose SHA-256 differs, and skips a build of the other kind.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class AppUpdateTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val server = MockWebServer()
    private val requests: MutableList<String> = Collections.synchronizedList(mutableListOf())
    private val apk = "PK\u0003\u0004 a pretend newer TheVault".toByteArray()
    private var builds = JSONArray()

    @Before
    fun setUp() {
        PairingStore.defaultCipher = FakeCipher
        WorkManagerTestInitHelper.initializeTestWorkManager(
            context,
            Configuration.Builder().setExecutor(SynchronousExecutor()).build(),
        )
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                val path = request.path ?: ""
                requests.add("${request.method} $path")
                return when {
                    path == "/api/clients" -> MockResponse()
                        .setHeader("content-type", "application/json")
                        .setBody(JSONObject().put("clients", builds).put("enabled", true).toString())
                    path.startsWith("/api/clients/") -> MockResponse().setBody(Buffer().write(apk))
                    else -> MockResponse().setResponseCode(404)
                }
            }
        }
        server.start()
        PairingStore(context).save(
            JSONObject()
                .put("server_url", server.url("/").toString())
                .put("device_id", "dev-1")
                .put("device_name", "Pixel")
                .put("token", "t"),
        )
        AppUpdateWorker.abisOverride = listOf("arm64-v8a", "armeabi-v7a")
        AppInstaller.lastParams = null
        installed("0.1.0", debuggable = false)
        shadowOf(context.packageManager).setCanRequestPackageInstalls(true)
    }

    @After
    fun tearDown() {
        AppUpdateWorker.abisOverride = null
        server.shutdown()
    }

    private fun installed(version: String, debuggable: Boolean) {
        val info = shadowOf(context.packageManager).getInternalMutablePackageInfo(context.packageName)
        info.versionName = version
        info.longVersionCode = 1_000L
        val flags = context.applicationInfo.flags
        context.applicationInfo.flags = if (debuggable) flags or ApplicationInfo.FLAG_DEBUGGABLE else flags and ApplicationInfo.FLAG_DEBUGGABLE.inv()
        info.applicationInfo?.flags = context.applicationInfo.flags
    }

    private fun sha(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    private fun publish(version: String, arch: String = "aarch64", build: String = "release", sha256: String = sha(apk)) {
        val suffix = if (build == "debug") "-debug" else ""
        val name = "TheVault_${version}_$arch$suffix.apk"
        builds.put(
            JSONObject()
                .put("platform", "android").put("kind", "apk").put("arch", arch).put("version", version)
                .put("build", build).put("size", apk.size).put("sha256", sha256).put("filename", name)
                .put("published", 1).put("path", "/api/clients/$name"),
        )
    }

    private fun check(): ListenableWorker.Result =
        TestWorkerBuilder.from(context, AppUpdateWorker::class.java, Executors.newSingleThreadExecutor()).build().doWork()

    private fun sessions() = context.packageManager.packageInstaller.allSessions

    private fun updateDir() = File(context.cacheDir, "updates")

    private fun notices() = shadowOf(context.getSystemService(NotificationManager::class.java)).allNotifications

    @Test
    fun aClientsUpdatedPushSchedulesTheUpdateCheck() {
        val push = JSONObject().put("event", "clients.updated").toString().toByteArray()
        assertEquals("clients.updated", PushHandler.handle(context, push))
        val work = WorkManager.getInstance(context).getWorkInfosForUniqueWork(AppUpdateWorker.NOW).get()
        assertEquals(1, work.size)
        assertEquals(WorkInfo.State.ENQUEUED, work.single().state)
        assertEquals(NetworkType.CONNECTED, work.single().constraints.requiredNetworkType)
        assertFalse(work.single().constraints.requiresCharging())
        assertFalse(work.single().constraints.requiresDeviceIdle())
    }

    @Test
    fun theAppSchedulesADailyCheckAndOneNow() {
        AppUpdateWorker.start(context)
        val wm = WorkManager.getInstance(context)
        assertEquals(1, wm.getWorkInfosForUniqueWork(AppUpdateWorker.DAILY).get().size)
        assertEquals(1, wm.getWorkInfosForUniqueWork(AppUpdateWorker.NOW).get().size)
    }

    @Test
    fun aNewerBuildIsDownloadedVerifiedAndInstalledWithoutUserAction() {
        publish("0.2.0", arch = "x86_64")
        publish("0.2.0")
        val result = check()
        assertTrue("$result", result is ListenableWorker.Result.Success)
        assertEquals(AppUpdateWorker.INSTALLING, result.outputData.getString(AppUpdateWorker.STATUS))
        assertEquals(listOf("GET /api/clients", "GET /api/clients/TheVault_0.2.0_aarch64.apk"), requests.toList())
        val session = sessions().single()
        assertEquals(context.packageName, session.appPackageName)
        val params = AppInstaller.lastParams!!
        val requireUserAction = PackageInstaller.SessionParams::class.java.getDeclaredField("requireUserAction")
            .apply { isAccessible = true }.getInt(params)
        assertEquals(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED, requireUserAction)
        val mode = PackageInstaller.SessionParams::class.java.getDeclaredField("mode")
            .apply { isAccessible = true }.getInt(params)
        assertEquals(PackageInstaller.SessionParams.MODE_FULL_INSTALL, mode)
        assertTrue("the download is cleaned up", updateDir().listFiles().isNullOrEmpty())
    }

    @Test
    fun theInstallersResultIsShownAsANotice() {
        UpdateInstallReceiver().onReceive(
            context,
            Intent(AppInstaller.ACTION_STATUS)
                .putExtra(AppInstaller.EXTRA_VERSION, "0.2.0")
                .putExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_SUCCESS),
        )
        assertEquals("TheVault updated", notices().single().extras.getString("android.title"))
        UpdateInstallReceiver().onReceive(
            context,
            Intent(AppInstaller.ACTION_STATUS)
                .putExtra(AppInstaller.EXTRA_VERSION, "0.2.0")
                .putExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE_INCOMPATIBLE)
                .putExtra(PackageInstaller.EXTRA_STATUS_MESSAGE, "signatures do not match"),
        )
        val failed = notices().single()
        assertEquals("TheVault update failed", failed.extras.getString("android.title"))
        assertTrue(failed.extras.getCharSequence("android.text").toString().contains("signatures do not match"))
        UpdateInstallReceiver().onReceive(
            context,
            Intent(AppInstaller.ACTION_STATUS)
                .putExtra(AppInstaller.EXTRA_VERSION, "0.2.0")
                .putExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_PENDING_USER_ACTION)
                .putExtra(Intent.EXTRA_INTENT, Intent("android.content.pm.action.CONFIRM_INSTALL")),
        )
        assertEquals("Update TheVault to 0.2.0", notices().single().extras.getString("android.title"))
        shadowOf(Looper.getMainLooper()).idle()
    }

    @Test
    fun theAppMayUpdateItselfWithoutAskingTheUser() {
        // Without it Android 12 and later answer USER_ACTION_NOT_REQUIRED with a confirmation.
        val info = context.packageManager.getPackageInfo(context.packageName, android.content.pm.PackageManager.GET_PERMISSIONS)
        assertTrue(info.requestedPermissions!!.contains("android.permission.UPDATE_PACKAGES_WITHOUT_USER_ACTION"))
    }

    @Test
    fun theUpdatedAppSaysSoWhenTheUpdateWasItsOwn() {
        val prefs = context.getSharedPreferences(AppUpdateWorker.PREFS, Context.MODE_PRIVATE)
        // Replaced by someone else (adb, a store): no notice.
        PackageReplacedReceiver().onReceive(context, Intent(Intent.ACTION_MY_PACKAGE_REPLACED))
        assertTrue(notices().isEmpty())
        prefs.edit().putString(AppInstaller.PENDING, "0.2.0").commit()
        PackageReplacedReceiver().onReceive(context, Intent(Intent.ACTION_MY_PACKAGE_REPLACED))
        val notice = notices().single()
        assertEquals("TheVault updated", notice.extras.getString("android.title"))
        assertEquals(UpdateNotices.CHANNEL, notice.channelId)
        assertEquals(R.drawable.ic_stat_thevault, notice.smallIcon.resId)
        assertNull(prefs.getString(AppInstaller.PENDING, null))
    }

    @Test
    fun theSameOrAnOlderVersionIsNeverInstalled() {
        publish("0.1.0")
        var result = check()
        assertEquals(AppUpdate.UP_TO_DATE, result.outputData.getString(AppUpdateWorker.STATUS))
        builds = JSONArray()
        publish("0.0.9")
        result = check()
        assertEquals(AppUpdate.UP_TO_DATE, result.outputData.getString(AppUpdateWorker.STATUS))
        assertTrue(sessions().isEmpty())
        assertEquals(listOf("GET /api/clients", "GET /api/clients"), requests.toList())
    }

    @Test
    fun aDownloadWhoseSha256DiffersIsDeletedAndNotInstalled() {
        publish("0.2.0", sha256 = "0".repeat(64))
        val result = check()
        assertTrue("$result", result is ListenableWorker.Result.Failure)
        assertEquals(AppUpdateWorker.SHA_MISMATCH, result.outputData.getString(AppUpdateWorker.STATUS))
        assertTrue(sessions().isEmpty())
        assertNull(AppInstaller.lastParams)
        assertTrue(updateDir().listFiles().isNullOrEmpty())
        assertEquals("TheVault update failed", notices().single().extras.getString("android.title"))
    }

    @Test
    fun aBuildOfTheOtherKindIsSkippedWithAReason() {
        installed("0.1.0", debuggable = true)
        publish("0.2.0", build = "release")
        val result = check()
        assertEquals(AppUpdate.BUILD_MISMATCH, result.outputData.getString(AppUpdateWorker.STATUS))
        assertTrue(result.outputData.getString(AppUpdateWorker.MESSAGE)!!.contains("debug build"))
        assertTrue(sessions().isEmpty())
        assertEquals(AppUpdate.BUILD_MISMATCH, AppUpdateWorker.lastStatus(context)!!.first)
    }

    @Test
    fun withoutTheInstallPermissionItAsksOnceAndDownloadsNothing() {
        shadowOf(context.packageManager).setCanRequestPackageInstalls(false)
        publish("0.2.0")
        val result = check()
        assertEquals(AppUpdateWorker.NEEDS_PERMISSION, result.outputData.getString(AppUpdateWorker.STATUS))
        assertEquals(listOf("GET /api/clients"), requests.toList())
        val notice = notices().single()
        assertEquals("TheVault 0.2.0 is ready", notice.extras.getString("android.title"))
        assertEquals(
            UpdateSetupActivity::class.java.name,
            shadowOf(notice.contentIntent).savedIntent.component!!.className,
        )
    }

    @Test
    fun versionsCompareAsSemver() {
        assertEquals(1, AppUpdate.compare("0.10.0", "0.9.0"))
        assertEquals(0, AppUpdate.compare("1.0.0+7", "v1.0.0"))
        assertEquals(-1, AppUpdate.compare("1.0.0-rc.1", "1.0.0"))
        assertEquals(-1, AppUpdate.compare("1.0.0-rc.2", "1.0.0-rc.10"))
        assertNull(AppUpdate.compare("1.0", "1.0.0"))
    }

    @Test
    fun theBuildForThePhonesFirstAbiIsPicked() {
        fun b(arch: String) = ClientBuild(arch, "0.2.0", "release", "x", 1, "TheVault_0.2.0_$arch.apk", "/api/clients/x")
        val all = listOf(b("x86_64"), b("armv7"), b("universal"))
        assertEquals("armv7", AppUpdate.pick(all, listOf("arm64-v8a", "armeabi-v7a"))!!.arch)
        assertEquals("universal", AppUpdate.pick(all, listOf("arm64-v8a"))!!.arch)
        assertNull(AppUpdate.pick(listOf(b("x86_64")), listOf("arm64-v8a")))
    }
}
