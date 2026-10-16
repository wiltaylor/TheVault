package dev.wiltaylor.thevault.device

import android.Manifest
import android.content.Context
import android.provider.CallLog.Calls
import android.provider.Telephony.Sms
import androidx.test.core.app.ApplicationProvider
import androidx.work.Configuration
import androidx.work.testing.SynchronousExecutor
import androidx.work.testing.WorkManagerTestInitHelper
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.io.File

/**
 * S5: importing the phone's history again (D7, D22). The providers are the fake ones, the core
 * and the default device scripts the real ones, through JNI. An import reads only the kinds and
 * dates asked for, files each message and call that has no note, never files one twice, and
 * reports its progress; the `import_history` command from Settings > Devices runs the same import
 * and reports it to the home server.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class ImportHistoryTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val home = FakeHome()
    private lateinit var phone: FakePhone
    private lateinit var capture: DeviceCapture
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
        phone = FakePhone(context)
        // Last year, last month, and today, in milliseconds.
        phone.sms(101, "+61412345678", "last year", Sms.MESSAGE_TYPE_INBOX, date = 1_000_000)
        phone.sms(102, "+61412345678", "last month", Sms.MESSAGE_TYPE_INBOX, date = 50_000_000)
        phone.sms(103, "+61412345678", "sent today", Sms.MESSAGE_TYPE_SENT, date = 90_000_000)
        phone.call(201, "+61412345678", Calls.INCOMING_TYPE, 30, date = 60_000_000)
        capture = phone.install()
        app.grantPermissions(Manifest.permission.READ_SMS, Manifest.permission.READ_CALL_LOG, Manifest.permission.READ_CONTACTS)
        assertTrue(Device.openCore(context))
        ImportState.reset()
    }

    @After
    fun tearDown() {
        ImportState.reset()
        Device.reset()
        home.close()
    }

    /** The note files of [type] in the local vault that carry [deviceRef]. */
    private fun notes(type: String, deviceRef: String): List<File> =
        File(Vault.dir(context), "notes/$type").listFiles().orEmpty().filter { it.readText().contains("\"$deviceRef\"") }

    private fun import(kinds: List<String>, from: Long?, to: Long?): JSONObject {
        CaptureWork.importHistory(context, kinds, from, to)
        val state = ImportState.current()
        assertFalse(state.toString(), state.getBoolean("running"))
        return state.getJSONObject("report")
    }

    private fun kind(report: JSONObject, name: String): JSONObject {
        val kinds = report.getJSONArray("kinds")
        return (0 until kinds.length()).map { kinds.getJSONObject(it) }.single { it.getString("kind") == name }
    }

    @Test
    fun anImportReadsTheChosenKindsAndDatesAndReportsAsItGoes() {
        val heard = mutableListOf<JSONObject>()
        ImportState.listener = { heard.add(it) }

        val report = import(listOf("sms"), 40_000_000, 100_000_000)

        val sms = kind(report, "sms")
        assertEquals(sms.toString(), 2, sms.getInt("found"))
        assertEquals(2, sms.getInt("imported"))
        assertEquals(1, report.getJSONArray("kinds").length())
        assertEquals(1, notes("message", "sms:102").size)
        assertEquals(1, notes("message", "sms:103").size)
        assertTrue("before the range", notes("message", "sms:101").isEmpty())
        assertTrue("calls were not asked for", notes("call", "call:201").isEmpty())
        // Queued, started, a page, the kind done, the end; then finished.
        assertTrue(heard.size.toString(), heard.size >= 4)
        assertTrue(heard.any { it.optBoolean("running") && it.has("report") && !it.getJSONObject("report").getBoolean("done") })
        assertTrue(heard.last().getJSONObject("report").getBoolean("done"))
        assertFalse(heard.last().getBoolean("running"))
    }

    @Test
    fun anItemThatHasANoteIsLeftAloneWhateverTheCursorsSay() {
        // Live capture filed SMS 102: its cursor is past it, and it has a note.
        VaultCore.enqueue("sms_received", capture.sms.get("sms:102").toString(), System.currentTimeMillis())
        VaultCore.runPending()
        assertEquals(1, notes("message", "sms:102").size)

        val first = kind(import(listOf("sms", "calls"), null, null), "sms")
        assertEquals(3, first.getInt("found"))
        assertEquals(1, first.getInt("existing"))
        assertEquals(2, first.getInt("imported"))

        val again = import(listOf("sms", "calls"), null, null)
        assertEquals(3, kind(again, "sms").getInt("existing"))
        assertEquals(1, kind(again, "calls").getInt("existing"))
        for (ref in listOf("sms:101", "sms:102", "sms:103")) assertEquals(ref, 1, notes("message", ref).size)
        assertEquals(1, notes("call", "call:201").size)
    }

    @Test
    fun anMmsThatHasANoteIsNotReadAgain() {
        phone.mms(301, "+61412345678", listOf(FakePhone.SELF), dateSeconds = 70_000)
        phone.part(3011, 301, "text/plain", text = "Look at this")
        val everything = 0L..Long.MAX_VALUE
        val mms = capture.history("mms", everything, null).items.single()
        assertFalse(mms.known)
        VaultCore.enqueue("mms_received", mms.data.toString(), System.currentTimeMillis())
        VaultCore.runPending()
        assertEquals(1, notes("message", "mms:301").size)

        // Read for an import, it comes as its ref alone: its parts are not read again.
        val again = capture.history("mms", everything, null) { VaultCore.noted(it) == "true" }.items.single()
        assertTrue(again.known)
        assertEquals("""{"device_ref":"mms:301"}""", again.data.toString())

        val report = kind(import(listOf("mms"), null, null), "mms")
        assertEquals(1, report.getInt("existing"))
        assertEquals(0, report.getInt("imported"))
        assertEquals(1, notes("message", "mms:301").size)
    }

    @Test
    fun aMissingPermissionIsReportedForItsKind() {
        app.denyPermissions(Manifest.permission.READ_CALL_LOG)
        val report = import(listOf("sms", "calls"), null, null)
        assertEquals(Manifest.permission.READ_CALL_LOG, kind(report, "calls").getString("missing_permission"))
        assertEquals(3, kind(report, "sms").getInt("imported"))
        assertTrue(notes("call", "call:201").isEmpty())
    }

    @Test
    fun theImportHistoryCommandRunsTheImportAndReportsItToTheHomeServer() {
        home.settings = JSONObject().put("commands", JSONArray().put("import_history"))
        home.command(
            "01CMDIMPORT",
            "import_history",
            JSONObject().put("kinds", JSONArray().put("calls")).put("since", "1970-01-01T00:00:00Z"),
        )

        CommandWorker.enqueue(context)

        val done = home.command("01CMDIMPORT")
        assertEquals(done.toString(), "done", done.getString("status"))
        val result = done.getJSONObject("result")
        assertEquals("calls", result.getJSONArray("kinds").getJSONObject(0).getString("kind"))
        assertEquals(1, result.getJSONArray("kinds").getJSONObject(0).getInt("imported"))
        assertTrue(result.getString("message"), result.getString("message").startsWith("calls: 1 found"))
        assertEquals(1, notes("call", "call:201").size)
        assertTrue("no SMS were asked for", notes("message", "sms:101").isEmpty())
        // It was taken as delivered, reported its progress while it ran, then its result.
        assertTrue(home.bodies.contains("""{"status":"delivered"}"""))
        assertTrue(home.bodies.toString(), home.bodies.any { it.contains("\"status\":\"delivered\"") && it.contains("\"progress\"") })
    }

    @Test
    fun theCommandFailsSayingWhichPermissionIsMissing() {
        app.denyPermissions(Manifest.permission.READ_CALL_LOG)
        home.settings = JSONObject().put("commands", JSONArray().put("import_history"))
        home.command("01CMDNOPERM", "import_history", JSONObject().put("kinds", JSONArray().put("calls")))

        CommandWorker.enqueue(context)

        val failed = home.command("01CMDNOPERM")
        assertEquals("failed", failed.getString("status"))
        val message = failed.getJSONObject("result").getString("message")
        assertTrue(message, message.contains(Manifest.permission.READ_CALL_LOG))
    }
}
