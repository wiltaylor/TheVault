package dev.wiltaylor.thevault.device

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.service.notification.StatusBarNotification
import androidx.test.core.app.ApplicationProvider
import androidx.work.Configuration
import androidx.work.ListenableWorker
import androidx.work.testing.SynchronousExecutor
import androidx.work.testing.TestWorkerBuilder
import androidx.work.testing.WorkManagerTestInitHelper
import java.io.File
import java.util.concurrent.Executors
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * S5: a device event whose handler fails on every pass is announced as one phone notification,
 * however many fail in a burst, which opens Settings > Devices > This device; the bell's copy of
 * it from the home server is not shown a second time (ticket 88). The core and its device
 * runtime are the real ones, through JNI, with a `call_ended` handler that always fails.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class CaptureProblemsTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val home = FakeHome()
    private val manager = context.getSystemService(NotificationManager::class.java)

    @Before
    fun setUp() {
        PairingStore.defaultCipher = FakeCipher
        WorkManagerTestInitHelper.initializeTestWorkManager(
            context,
            Configuration.Builder().setExecutor(SynchronousExecutor()).build(),
        )
        shadowOf(context as android.app.Application).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
        PairingStore(context).save(
            JSONObject()
                .put("server_url", home.url)
                .put("device_id", FakeHome.DEVICE_ID)
                .put("device_name", "Pixel")
                .put("token", FakeHome.TOKEN),
        )
        val scripts = File(Vault.dir(context), "scripts").apply { mkdirs() }
        File(scripts, "device_calls.ws").writeText(
            "fn call_ended(event: Value) -> Result[(), string] { Err(\"no call type\") }\n",
        )
        FakePhone(context).install()
        assertTrue(Device.openCore(context))
    }

    @After
    fun tearDown() {
        Device.reset()
        home.close()
    }

    private fun call(ref: String, name: String) {
        val data = JSONObject()
            .put("device_ref", ref)
            .put("direction", "in")
            .put("number", "+61412345678")
            .put("number_raw", "0412 345 678")
            .put("contact", JSONObject().put("id", "3").put("name", name))
            .put("started", "2026-10-20T12:00:00+11:00")
            .put("duration", 30)
        VaultCore.enqueue("call_ended", data.toString(), 1_760_000_000_000)
    }

    private fun runEvents(): ListenableWorker.Result =
        TestWorkerBuilder.from(context, RunEventsWorker::class.java, Executors.newSingleThreadExecutor())
            .build()
            .doWork()

    private fun problems(): List<StatusBarNotification> =
        manager.activeNotifications.filter { it.notification.channelId == CaptureProblems.CHANNEL }

    private fun text(n: StatusBarNotification) = n.notification.extras.getCharSequence(Notification.EXTRA_TEXT).toString()

    @Test
    fun aBurstOfFailuresIsOneNotificationThatOpensThisDevice() {
        call("call:1", "Ada Lovelace")
        call("call:2", "Grace Hopper")
        // Two failed passes leave the events queued: nothing is announced.
        runEvents()
        runEvents()
        assertTrue(problems().isEmpty())

        runEvents()
        val shown = problems()
        assertEquals(1, shown.size)
        val n = shown.single()
        assertEquals(CaptureProblems.NOTIFICATION_ID, n.id)
        assertEquals(CaptureProblems.GROUP, n.notification.group)
        assertEquals("A message could not be saved", n.notification.extras.getString(Notification.EXTRA_TITLE))
        assertTrue(text(n), text(n).startsWith("2 "))
        assertEquals(2, n.notification.number)
        val tap = shadowOf(n.notification.contentIntent).savedIntent
        assertEquals("/settings/devices#this-device", tap.getStringExtra(NotificationMirror.EXTRA_PATH))

        // Announced once: another pass adds nothing.
        runEvents()
        assertEquals(2, problems().single().notification.number)

        // A later failure while it is still showing joins the same notification.
        call("call:3", "Alan Turing")
        repeat(3) { runEvents() }
        val again = problems()
        assertEquals(1, again.size)
        assertEquals(3, again.single().notification.number)
        assertTrue(text(again.single()), text(again.single()).startsWith("3 "))

        // Once dismissed from the shade, the next failure starts a count of its own.
        manager.cancelAll()
        call("call:4", "Ada Lovelace")
        repeat(3) { runEvents() }
        val alone = problems().single()
        assertEquals(1, alone.notification.number)
        val line = text(alone)
        assertTrue(line, line.startsWith("A call from Ada Lovelace was not saved: ") && line.contains("no call type"))
    }

    @Test
    fun theBellsCopyOfThisPhonesFailureIsNotMirrored() {
        home.notify(
            "01NOTIFOWN0000000000000000",
            "A call could not be saved",
            url = "/settings/devices#this-device",
            source = "device:${FakeHome.DEVICE_ID}",
        )
        home.notify("01NOTIFOTHER00000000000000", "From elsewhere", source = "device:01OTHERDEVICE0000000000000")
        // Past the first reconcile, which only notes what is already unread.
        NotificationMirror.reconcile(context, JSONArray())
        NotificationMirror.reconcile(context, JSONArray(VaultCore.unreadNotifications()))
        assertEquals(setOf("01NOTIFOTHER00000000000000"), NotificationMirror.shown(context))
    }
}
