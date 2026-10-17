package dev.wiltaylor.thevault.device

import android.Manifest
import android.app.Application
import android.app.NotificationManager
import android.content.Intent
import android.net.Uri
import android.os.PowerManager
import android.provider.Settings
import androidx.test.core.app.ApplicationProvider
import androidx.work.ListenableWorker
import androidx.work.testing.TestWorkerBuilder
import androidx.work.testing.WorkManagerTestInitHelper
import java.util.concurrent.Executors
import org.json.JSONArray
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
import org.robolectric.shadows.ShadowBuild

/**
 * S5: the Capture health panel's states (ticket 91): a capture kind whose permission is missing,
 * blocked by restricted settings, or not set up; battery optimisation, with the Samsung line;
 * background work that has not run for two hours; the one notification a persistent problem
 * raises at most once a day; and the health the phone sends with its settings sync.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class CaptureHealthTest {
    private val app: Application = ApplicationProvider.getApplicationContext()
    private val manager = app.getSystemService(NotificationManager::class.java)
    private val homes = mutableListOf<FakeHome>()

    @Before
    fun setUp() {
        PairingStore.defaultCipher = FakeCipher
        WorkManagerTestInitHelper.initializeTestWorkManager(app)
        shadowOf(app).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
    }

    @After
    fun tearDown() {
        Device.reset()
        homes.forEach { it.close() }
    }

    private fun pair(url: String = "http://127.0.0.1:9") {
        PairingStore(app).save(
            JSONObject()
                .put("server_url", url)
                .put("device_id", FakeHome.DEVICE_ID)
                .put("device_name", "Pixel")
                .put("token", FakeHome.TOKEN),
        )
    }

    private fun optimised(on: Boolean) {
        shadowOf(app.getSystemService(PowerManager::class.java)).setIgnoringBatteryOptimizations(app.packageName, !on)
    }

    private fun fromPlayStore() {
        shadowOf(app.packageManager).setInstallSourceInfo(app.packageName, "com.android.vending", "com.android.vending")
    }

    private fun kind(health: JSONObject, id: String): JSONObject {
        val kinds = health.getJSONArray("kinds")
        return (0 until kinds.length()).map { kinds.getJSONObject(it) }.first { it.getString("kind") == id }
    }

    private fun work(health: JSONObject, id: String): JSONObject {
        val works = health.getJSONArray("background")
        return (0 until works.length()).map { works.getJSONObject(it) }.first { it.getString("work") == id }
    }

    private fun strings(a: JSONArray) = (0 until a.length()).map { a.getString(it) }

    private fun problems(health: JSONObject) = strings(health.getJSONArray("problems"))

    private fun shown() = manager.activeNotifications.filter { it.notification.channelId == CaptureHealth.CHANNEL }

    @Test
    fun aMissingPermissionIsAProblemWithAButtonForAppInfo() {
        optimised(false)
        fromPlayStore()
        // Before capture setup is finished, nothing is a problem yet.
        var health = CaptureHealth.check(app)
        assertEquals(CaptureHealth.NOT_SET_UP, kind(health, "sms").getString("state"))
        assertEquals(CaptureHealth.FIX_CAPTURE_SETUP, kind(health, "sms").getString("fix"))
        assertEquals(emptyList<String>(), problems(health))

        Device.markSetupDone(app)
        health = CaptureHealth.check(app)
        val sms = kind(health, "sms")
        assertEquals(CaptureHealth.PERMISSION_MISSING, sms.getString("state"))
        assertEquals(CaptureHealth.FIX_APP_DETAILS, sms.getString("fix"))
        assertTrue(sms.getString("message"), sms.getString("message").contains("READ_SMS"))
        assertTrue(problems(health).toString(), "SMS: permission missing" in problems(health))
        assertTrue("sms.permission" in strings(health.getJSONArray("persistent")))
        // RCS from Play: notification access is off, and the button opens it.
        val rcs = kind(health, "rcs")
        assertEquals(CaptureHealth.PERMISSION_MISSING, rcs.getString("state"))
        assertEquals(CaptureHealth.FIX_NOTIFICATION_ACCESS, rcs.getString("fix"))

        shadowOf(app).grantPermissions(Manifest.permission.RECEIVE_SMS, Manifest.permission.READ_SMS)
        health = CaptureHealth.check(app)
        assertEquals(CaptureHealth.WORKING, kind(health, "sms").getString("state"))
        assertEquals(CaptureHealth.WORKING, kind(health, "mms").getString("state"))
        assertFalse("SMS: permission missing" in problems(health))
        assertTrue("Calls: permission missing" in problems(health))

        // A step the user skipped is not set up, and not a problem.
        CaptureHealth.skipped(app, CaptureHealth.STEP_KINDS.getValue("Calls"))
        health = CaptureHealth.check(app)
        assertEquals(CaptureHealth.NOT_SET_UP, kind(health, "calls").getString("state"))
        assertFalse("Calls: permission missing" in problems(health))
    }

    @Test
    fun notificationAccessForASideloadedAppIsBlockedByRestrictedSettings() {
        optimised(false)
        Device.markSetupDone(app)
        // Installed by adb: no installer, as for an APK from the home server.
        val rcs = kind(CaptureHealth.check(app), "rcs")
        assertEquals(CaptureHealth.RESTRICTED, rcs.getString("state"))
        assertEquals(CaptureHealth.FIX_APP_DETAILS, rcs.getString("fix"))
        assertEquals(CaptureHealth.FIX_NOTIFICATION_ACCESS, rcs.getString("then"))
        assertTrue(problems(CaptureHealth.check(app)).contains("RCS: blocked by restricted settings"))
        // Android 13 does not restrict SMS, even when a request was refused.
        CaptureHealth.blocked(app, listOf("sms"))
        assertEquals(CaptureHealth.PERMISSION_MISSING, kind(CaptureHealth.check(app), "sms").getString("state"))

        Settings.Secure.putString(app.contentResolver, "enabled_notification_listeners", RcsListenerService.component(app).flattenToString())
        assertEquals(CaptureHealth.WORKING, kind(CaptureHealth.check(app), "rcs").getString("state"))
    }

    @Test
    @Config(sdk = [35])
    fun smsRefusedWithoutADialogOnAndroid15IsBlockedByRestrictedSettings() {
        optimised(false)
        Device.markSetupDone(app)
        assertEquals(CaptureHealth.PERMISSION_MISSING, kind(CaptureHealth.check(app), "sms").getString("state"))
        CaptureHealth.blocked(app, CaptureHealth.STEP_KINDS.getValue("Messages"))
        val health = CaptureHealth.check(app)
        val sms = kind(health, "sms")
        assertEquals(CaptureHealth.RESTRICTED, sms.getString("state"))
        assertEquals(CaptureHealth.FIX_APP_DETAILS, sms.getString("fix"))
        assertTrue(sms.getString("message"), sms.getString("message").contains("Allow restricted settings"))
        assertTrue("SMS: blocked by restricted settings" in problems(health))
        assertTrue("sms.permission" in strings(health.getJSONArray("persistent")))
        // From Play, restricted settings do not apply.
        fromPlayStore()
        assertEquals(CaptureHealth.PERMISSION_MISSING, kind(CaptureHealth.check(app), "sms").getString("state"))
    }

    @Test
    fun eachProblemOpensItsExactSystemScreen() {
        val details = CaptureHealth.fixIntent(app, CaptureHealth.FIX_APP_DETAILS)!!
        assertEquals(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, details.action)
        assertEquals(Uri.fromParts("package", app.packageName, null), details.data)
        val listener = CaptureHealth.fixIntent(app, CaptureHealth.FIX_NOTIFICATION_ACCESS)!!
        assertEquals(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS, listener.action)
        assertEquals(
            RcsListenerService.component(app).flattenToString(),
            listener.getStringExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME),
        )
        val battery = CaptureHealth.fixIntent(app, CaptureHealth.FIX_BATTERY)!!
        assertEquals(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, battery.action)
        assertEquals(Uri.fromParts("package", app.packageName, null), battery.data)
        val setup = CaptureHealth.fixIntent(app, CaptureHealth.FIX_CAPTURE_SETUP)!!
        assertEquals(CaptureSetupActivity::class.java.name, setup.component!!.className)
        assertNull(CaptureHealth.fixIntent(app, "reboot"))
    }

    @Test
    fun batteryOptimisationIsAProblemWithTheExemptionButtonAndTheSamsungLine() {
        optimised(true)
        var health = CaptureHealth.check(app)
        val battery = health.getJSONObject("battery")
        assertTrue(battery.getBoolean("optimised"))
        assertEquals(CaptureHealth.FIX_BATTERY, battery.getString("fix"))
        assertFalse(battery.getBoolean("samsung"))
        assertFalse(battery.has("samsung_line"))
        assertTrue("Battery optimisation is on" in problems(health))
        assertTrue("battery" in strings(health.getJSONArray("persistent")))
        assertTrue(CaptureHealth.report(health).getBoolean("battery_optimised"))

        ShadowBuild.setManufacturer("samsung")
        health = CaptureHealth.check(app)
        assertEquals(CaptureHealth.SAMSUNG_LINE, health.getJSONObject("battery").getString("samsung_line"))
        assertTrue(CaptureHealth.SAMSUNG_LINE.contains("Sleeping apps"))

        optimised(false)
        health = CaptureHealth.check(app)
        assertFalse(health.getJSONObject("battery").getBoolean("optimised"))
        assertTrue(health.getJSONObject("battery").isNull("fix"))
        assertFalse("Battery optimisation is on" in problems(health))
    }

    @Test
    fun backgroundWorkThatHasNotRunForTwoHoursWhilePairedIsStalled() {
        optimised(false)
        Device.markSetupDone(app)
        val t0 = 1_760_000_000_000L
        val hour = 60 * 60 * 1000L
        // Not paired: nothing is expected to run.
        var health = CaptureHealth.check(app, t0)
        assertFalse(work(health, CaptureHealth.CATCH_UP).getBoolean("expected"))
        assertFalse(work(CaptureHealth.check(app, t0 + 5 * hour), CaptureHealth.CATCH_UP).getBoolean("stale"))

        pair()
        health = CaptureHealth.check(app, t0)
        assertTrue(work(health, CaptureHealth.CATCH_UP).getBoolean("expected"))
        assertFalse(work(health, CaptureHealth.CATCH_UP).getBoolean("stale"))
        // The command check is watched only on the 15-minute schedule, with no push distributor.
        assertFalse(work(health, CaptureHealth.COMMANDS).getBoolean("expected"))

        // Never ran, two hours on.
        health = CaptureHealth.check(app, t0 + 2 * hour + 1)
        val catchUp = work(health, CaptureHealth.CATCH_UP)
        assertTrue(catchUp.getBoolean("stale"))
        assertTrue(catchUp.isNull("last"))
        assertTrue(problems(health).toString(), problems(health).any { it.startsWith("Catch-up") && it.contains("2 hours") })
        assertTrue("background.catch_up" in strings(health.getJSONArray("persistent")))
        assertTrue(CaptureHealth.report(health).getJSONArray("background").getJSONObject(0).getBoolean("stale"))

        CaptureHealth.ran(app, CaptureHealth.CATCH_UP, t0 + 2 * hour)
        health = CaptureHealth.check(app, t0 + 3 * hour)
        assertFalse(work(health, CaptureHealth.CATCH_UP).getBoolean("stale"))
        assertEquals(t0 + 2 * hour, work(health, CaptureHealth.CATCH_UP).getLong("last"))
        health = CaptureHealth.check(app, t0 + 5 * hour)
        assertTrue(work(health, CaptureHealth.CATCH_UP).getBoolean("stale"))
        assertTrue(problems(health).toString(), problems(health).any { it.contains("3 hours") })

        CaptureHealth.setCommandsPolled(app, true)
        CaptureHealth.ran(app, CaptureHealth.COMMANDS, t0 + 3 * hour)
        assertFalse(work(CaptureHealth.check(app, t0 + 4 * hour), CaptureHealth.COMMANDS).getBoolean("stale"))
        // Watched from t0 + 4 hours, the first check on the schedule: stalled once two hours pass after it.
        assertFalse(work(CaptureHealth.check(app, t0 + 6 * hour), CaptureHealth.COMMANDS).getBoolean("stale"))
        assertTrue(work(CaptureHealth.check(app, t0 + 6 * hour + 1), CaptureHealth.COMMANDS).getBoolean("stale"))
        CaptureHealth.setCommandsPolled(app, false)
        assertFalse(work(CaptureHealth.check(app, t0 + 7 * hour), CaptureHealth.COMMANDS).getBoolean("stale"))
    }

    @Test
    fun theCatchUpWorkerRecordsThatItRan() {
        FakePhone(app).install()
        val before = System.currentTimeMillis()
        val result = TestWorkerBuilder.from(app, CatchUpWorker::class.java, Executors.newSingleThreadExecutor()).build().doWork()
        assertTrue("$result", result is ListenableWorker.Result.Success)
        assertTrue(CaptureHealth.lastRan(app, CaptureHealth.CATCH_UP)!! >= before)
    }

    @Test
    fun aPersistentProblemRaisesOneNotificationThatOpensThePanelAtMostOnceADay() {
        optimised(true)
        val t0 = 1_760_000_000_000L
        val hour = 60 * 60 * 1000L
        CaptureHealth.refresh(app, now = t0)
        val n = shown().single()
        assertEquals(CaptureHealth.NOTIFICATION_ID, n.id)
        assertEquals(CaptureHealth.TITLE, n.notification.extras.getString(android.app.Notification.EXTRA_TITLE))
        val tap: Intent = shadowOf(n.notification.contentIntent).savedIntent
        assertEquals("/settings/devices#capture-health", tap.getStringExtra(NotificationMirror.EXTRA_PATH))

        // Dismissed, it does not come back the same day.
        manager.cancelAll()
        CaptureHealth.refresh(app, now = t0 + hour)
        assertTrue(shown().isEmpty())
        // The panel's own check never raises it.
        CaptureHealth.refresh(app, notify = false, now = t0 + 25 * hour)
        assertTrue(shown().isEmpty())

        // A new problem raises it within the day.
        Device.markSetupDone(app)
        CaptureHealth.refresh(app, now = t0 + 2 * hour)
        val again = shown().single()
        val text = again.notification.extras.getCharSequence(android.app.Notification.EXTRA_BIG_TEXT).toString()
        assertTrue(text, text.contains("SMS: permission missing") && text.contains("Battery optimisation is on"))

        // A day on, the battery raises it again.
        manager.cancelAll()
        CaptureHealth.refresh(app, now = t0 + 24 * hour)
        assertNotNull(shown().singleOrNull())

        // Nothing wrong: nothing raised.
        manager.cancelAll()
        optimised(false)
        shadowOf(app).grantPermissions(*CaptureHealth.KINDS.flatMap { it.permissions }.toTypedArray())
        Settings.Secure.putString(app.contentResolver, "enabled_notification_listeners", RcsListenerService.component(app).flattenToString())
        CaptureHealth.refresh(app, now = t0 + 60 * hour)
        assertTrue(shown().isEmpty())
    }

    @Test
    fun theHealthGoesToTheHomeServerWithTheSettingsSync() {
        optimised(true)
        val home = FakeHome().also { homes.add(it) }
        pair(home.url)
        Device.markSetupDone(app)
        val result = TestWorkerBuilder.from(app, SyncWorker::class.java, Executors.newSingleThreadExecutor()).build().doWork()
        assertTrue("$result", result is ListenableWorker.Result.Success)
        val requests = home.requests.toList()
        val health = "PUT /api/devices/${FakeHome.DEVICE_ID}/health"
        assertEquals(requests.toString(), requests.indexOf("GET /api/devices") + 1, requests.indexOf(health))
        val sent = home.health!!
        val problems = strings(sent.getJSONArray("problems"))
        assertTrue(problems.toString(), "SMS: permission missing" in problems && "Battery optimisation is on" in problems)
        assertTrue(sent.getBoolean("battery_optimised"))
        // What the phone sends leaves out the times that change on every check.
        assertFalse(sent.has("checked"))

        // Unchanged, the next round does not send it again.
        home.requests.clear()
        TestWorkerBuilder.from(app, SyncWorker::class.java, Executors.newSingleThreadExecutor()).build().doWork()
        assertFalse(home.requests.toString(), health in home.requests)
    }
}
