package dev.wiltaylor.thevault.device

import android.Manifest
import android.content.Context
import android.telephony.SmsManager
import androidx.test.core.app.ApplicationProvider
import androidx.work.Configuration
import androidx.work.WorkManager
import androidx.work.testing.SynchronousExecutor
import androidx.work.testing.WorkManagerTestInitHelper
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.unifiedpush.android.connector.data.PushMessage

/**
 * S5: device commands (D22, D23). A `device.command` push, or the app opening, runs the pending
 * commands through the real core against a fake home server. `send_sms` goes out through
 * `SmsManager` only when the device file declares it, the user enabled it for the device, the
 * SMS-send switch is on and Android granted SEND_SMS; every result, sent or refused, is posted.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class DeviceCommandsTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val home = FakeHome()
    private val app get() = shadowOf(context as android.app.Application)

    @Before
    fun setUp() {
        PairingStore.defaultCipher = FakeCipher
        WorkManagerTestInitHelper.initializeTestWorkManager(
            context,
            Configuration.Builder().setExecutor(SynchronousExecutor()).build(),
        )
        PairingStore(context).save(
            JSONObject()
                .put("server_url", home.url)
                .put("device_id", FakeHome.DEVICE_ID)
                .put("device_name", "Pixel")
                .put("token", FakeHome.TOKEN),
        )
        phone.install()
    }

    private val phone by lazy { FakePhone(context) }

    /**
     * Android's side of the next send, on a thread of its own while the command blocks: once the
     * text is handed to SmsManager, write its sent row as [row] and report the send with [code].
     */
    private fun androidSends(row: Long, code: Int = android.app.Activity.RESULT_OK) = kotlin.concurrent.thread {
        val until = System.currentTimeMillis() + 10_000
        while (sent() == null && System.currentTimeMillis() < until) Thread.sleep(10)
        val params = sent() ?: return@thread
        phone.sms(row, params.destinationAddress, params.text, android.provider.Telephony.Sms.MESSAGE_TYPE_SENT)
        params.sentIntent.send(context, code, null)
    }

    @After
    fun tearDown() {
        Device.reset()
        home.close()
    }

    private fun settings(commands: List<String>, smsSend: Boolean) {
        home.settings = JSONObject().put("commands", JSONArray(commands)).put("sms_send", smsSend)
    }

    private fun sendSms(id: String) =
        home.command(id, "send_sms", JSONObject().put("to", "+61400000002").put("body", "Bins out"))

    /** What the distributor hands over once the connector has decrypted it. */
    private fun push(json: String) {
        Robolectric.buildService(VaultPushService::class.java).create().get()
            .onMessage(PushMessage(json.toByteArray(Charsets.UTF_8), true), "default")
    }

    private fun sent() = shadowOf(context.getSystemService(SmsManager::class.java)).lastSentTextMessageParams

    private fun result(id: String): JSONObject = home.command(id)

    @Test
    fun sendSmsRunsOnlyWhenEnabledAndBothSwitchesAllowItAndEveryResultIsPosted() {
        // Not enabled for this device.
        app.grantPermissions(Manifest.permission.SEND_SMS)
        settings(listOf("show_notification"), smsSend = true)
        sendSms("01CMD1")
        push("""{"event":"device.command","id":"01CMD1"}""")
        assertEquals("failed", result("01CMD1").getString("status"))
        assertTrue(result("01CMD1").getString("result"), result("01CMD1").getString("result").contains("is not enabled for this device"))
        assertNull(sent())

        // Enabled, but the device's SMS-send switch is off.
        settings(listOf("send_sms"), smsSend = false)
        sendSms("01CMD2")
        push("""{"event":"device.command","id":"01CMD2"}""")
        assertEquals("failed", result("01CMD2").getString("status"))
        assertTrue(result("01CMD2").getString("result").contains("turn on SMS send in Settings > Devices"))
        assertNull(sent())

        // Switched on, but Android has not granted SEND_SMS.
        app.denyPermissions(Manifest.permission.SEND_SMS)
        settings(listOf("send_sms"), smsSend = true)
        sendSms("01CMD3")
        push("""{"event":"device.command","id":"01CMD3"}""")
        assertEquals("failed", result("01CMD3").getString("status"))
        assertTrue(result("01CMD3").getString("result"), result("01CMD3").getString("result").contains("SEND_SMS"))
        assertNull(sent())

        // Everything allows it: the text goes, and once Android reports it sent, the result is
        // posted with the phone's id of the message (outbox X6).
        app.grantPermissions(Manifest.permission.SEND_SMS)
        sendSms("01CMD4")
        val android = androidSends(row = 4412)
        push("""{"event":"device.command","id":"01CMD4"}""")
        android.join()
        assertEquals(result("01CMD4").toString(), "done", result("01CMD4").getString("status"))
        assertEquals("sms:4412", result("01CMD4").getJSONObject("result").getString("device_ref"))
        assertEquals("+61400000002", sent().destinationAddress)
        assertEquals("Bins out", sent().text)

        // Each command was taken as delivered before it ran, then reported.
        val reports = home.requests.filter { it.startsWith("POST /api/devices/${FakeHome.DEVICE_ID}/commands/") }
        assertEquals(8, reports.size)
        assertTrue(home.bodies.contains("""{"status":"delivered"}"""))
    }

    @Test
    fun aCommandTheDeviceFileDoesNotDeclareIsRefused() {
        // This vault's device file accepts notifications only.
        val devices = java.io.File(Vault.dir(context), "devices").apply { mkdirs() }
        java.io.File(devices, "android.wcl").writeText("command \"show_notification\" {\n  script = \"device_messages\"\n}\n")
        app.grantPermissions(Manifest.permission.SEND_SMS)
        settings(listOf("send_sms"), smsSend = true)
        sendSms("01CMD5")

        CommandWorker.enqueue(context)

        assertEquals("failed", result("01CMD5").getString("status"))
        assertTrue(result("01CMD5").getString("result"), result("01CMD5").getString("result").contains("does not declare it"))
        assertNull(sent())
    }

    @Test
    fun theFallbackCheckRunsCommandsBesideNotifications() {
        NotificationWorker.schedule(context)
        val work = WorkManager.getInstance(context)
        assertEquals(1, work.getWorkInfosForUniqueWork(CommandWorker.FALLBACK).get().size)
        NotificationWorker.unschedule(context)
        assertTrue(work.getWorkInfosForUniqueWork(CommandWorker.FALLBACK).get().all { it.state.isFinished })
    }
}
