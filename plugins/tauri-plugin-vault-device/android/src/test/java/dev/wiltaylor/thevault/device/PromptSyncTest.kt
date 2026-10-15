package dev.wiltaylor.thevault.device

import android.content.Context
import android.content.Intent
import android.provider.CallLog.Calls
import android.provider.Telephony
import android.provider.Telephony.Sms
import android.telephony.TelephonyManager
import androidx.test.core.app.ApplicationProvider
import androidx.work.Configuration
import androidx.work.WorkInfo
import androidx.work.WorkManager
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
import org.robolectric.annotation.Config

/**
 * S5, ticket 78: a capture made in the background reaches the home server promptly. The receivers
 * hand events to the real core through JNI, the default device scripts write the notes, and the
 * notes' sync waits for neither the app opening nor the 15-minute period: [RunEventsWorker] asks
 * [SyncWorker.soon] for one expedited round that needs the network, and a burst of captures asks
 * for one round, not one each. The fake home server takes the pushed notes.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class PromptSyncTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val home = FakeHome()
    private lateinit var phone: FakePhone

    @Before
    fun setUp() {
        PairingStore.defaultCipher = FakeCipher
        WorkManagerTestInitHelper.initializeTestWorkManager(
            context,
            Configuration.Builder().setExecutor(SynchronousExecutor()).build(),
        )
        SyncWorker.settleMs = 0
        SyncWorker.foreground = false
        SyncWorker.resetPrompt()
        phone = FakePhone(context)
        phone.install(CoreSink(context)).begin()
    }

    @After
    fun tearDown() {
        SyncWorker.foreground = false
        Device.reset()
        home.close()
    }

    private fun pair() {
        PairingStore(context).save(
            JSONObject()
                .put("server_url", home.url)
                .put("device_id", FakeHome.DEVICE_ID)
                .put("device_name", "Pixel")
                .put("token", FakeHome.TOKEN),
        )
    }

    /** The prompt rounds WorkManager holds. */
    private fun rounds(): List<WorkInfo> =
        WorkManager.getInstance(context).getWorkInfosForUniqueWork(SyncWorker.SOON).get()

    private fun pending(): Int = JSONObject(VaultCore.status()).getInt("pending")

    /** The notes waiting for the home server, by type. */
    private fun queuedTypes(): List<String> {
        val queued = JSONArray(VaultCore.queued())
        return (0 until queued.length()).map { queued.getJSONObject(it).optString("type") }
    }

    private fun smsArrives(id: Long) {
        phone.sms(id, "+61412345678", "message $id", Sms.MESSAGE_TYPE_INBOX)
        SmsReceivedReceiver().onReceive(context, Intent(Telephony.Sms.Intents.SMS_RECEIVED_ACTION))
    }

    private fun callEnds(id: Long) {
        phone.call(id, "+61412345678", Calls.INCOMING_TYPE, 30)
        PhoneStateReceiver().onReceive(
            context,
            Intent(TelephonyManager.ACTION_PHONE_STATE_CHANGED)
                .putExtra(TelephonyManager.EXTRA_STATE, TelephonyManager.EXTRA_STATE_IDLE),
        )
    }

    /** The one round waiting, which needs the network and has not run. */
    private fun oneRoundWaiting(): WorkInfo {
        val rounds = rounds()
        assertEquals("one prompt round: $rounds", 1, rounds.size)
        val round = rounds.single()
        assertEquals(WorkInfo.State.ENQUEUED, round.state)
        return round
    }

    @Test
    fun aCapturedSmsSchedulesOnePromptSync() {
        pair()
        smsArrives(1)
        assertTrue("the handler wrote a message: ${queuedTypes()}", "message" in queuedTypes())
        oneRoundWaiting()
    }

    @Test
    fun anEndedCallSchedulesOnePromptSync() {
        pair()
        callEnds(1)
        assertTrue("the handler wrote a call: ${queuedTypes()}", "call" in queuedTypes())
        oneRoundWaiting()
    }

    @Test
    fun aBurstOfFiveCapturesCoalescesToOneRound() {
        pair()
        for (id in 1L..5L) smsArrives(id)
        assertEquals(5, queuedTypes().count { it == "message" })
        oneRoundWaiting()
    }

    @Test
    fun thePromptRoundRunsTheSyncEngineAndEmptiesTheQueue() {
        pair()
        smsArrives(1)
        assertTrue(pending() > 0)
        val round = oneRoundWaiting()

        // Offline it waits; once the network is up, WorkManager runs it.
        WorkManagerTestInitHelper.getTestDriver(context)!!.setAllConstraintsMet(round.id)

        assertEquals(WorkInfo.State.SUCCEEDED, rounds().single().state)
        assertEquals("every note reached the server", 0, pending())
        assertTrue("GET /api/sync/info" in home.requests)
        assertTrue(home.notes.values.any { it.contains("message 1") })

        // A capture after that round began asks for a round of its own.
        smsArrives(2)
        val after = rounds()
        assertEquals("$after", 2, after.size)
        assertEquals(setOf(WorkInfo.State.SUCCEEDED, WorkInfo.State.ENQUEUED), after.map { it.state }.toSet())
    }

    @Test
    fun anUnpairedDeviceSchedulesNoSync() {
        smsArrives(1)
        assertTrue("events wait for pairing", rounds().isEmpty())
        assertFalse(SyncWorker.soon(context))
        assertTrue(rounds().isEmpty())
        assertTrue("nothing left the phone", home.requests.isEmpty())
    }

    private fun notePush(vararg ids: String): String? =
        PushHandler.handle(
            context,
            JSONObject().put("event", "note.changed").put("ids", JSONArray(ids.toList()))
                .toString().toByteArray(),
        )

    /** Ticket 82: a note changed on the server, pushed while the app is closed, is pulled soon. */
    @Test
    fun aNoteChangedPushSchedulesOnePromptSync() {
        pair()
        assertEquals("note.changed", notePush("01JB8Z4K2MQ7V3X9T5R6W8Y2ZA"))
        assertEquals("note.changed", notePush("01JB8Z4K2MQ7V3X9T5R6W8Y2ZB"))
        val round = oneRoundWaiting()

        WorkManagerTestInitHelper.getTestDriver(context)!!.setAllConstraintsMet(round.id)

        assertEquals(WorkInfo.State.SUCCEEDED, rounds().single().state)
        assertTrue("the round ran: ${home.requests}", "GET /api/sync/info" in home.requests)
        assertTrue("the round pulled: ${home.requests}", home.requests.any { it.startsWith("GET /api/sync/changes") })
    }

    /**
     * Ticket 84: settings changed in Settings > Devices while the app is closed are pushed as
     * `device.settings`; the prompt round it schedules takes the settings before it syncs.
     */
    @Test
    fun aDeviceSettingsPushHasTheNextRoundTakeTheSettingsFirst() {
        pair()
        home.settings = JSONObject().put("contacts_write", true)
        val push = JSONObject().put("event", "device.settings").toString().toByteArray()
        assertEquals("device.settings", PushHandler.handle(context, push))
        val round = oneRoundWaiting()

        WorkManagerTestInitHelper.getTestDriver(context)!!.setAllConstraintsMet(round.id)

        assertEquals(WorkInfo.State.SUCCEEDED, rounds().single().state)
        val requests = home.requests.toList()
        val settings = requests.indexOf("GET /api/devices")
        assertTrue("the round took the settings: $requests", settings >= 0)
        assertTrue("before it synced: $requests", settings < requests.indexOf("GET /api/sync/info"))
    }

    @Test
    fun aNoteChangedPushToAnUnpairedPhoneSchedulesNothing() {
        assertEquals("note.changed", notePush("01JB8Z4K2MQ7V3X9T5R6W8Y2ZA"))
        assertTrue(rounds().isEmpty())
        assertTrue(home.requests.isEmpty())
    }

    @Test
    fun withTheAppInFrontTheForegroundLoopSyncsInstead() {
        pair()
        SyncWorker.foreground = true
        smsArrives(1)
        assertTrue("no WorkManager round", rounds().isEmpty())
        val deadline = System.currentTimeMillis() + 30_000
        while (pending() > 0) {
            assertTrue("the foreground loop ran no round: ${VaultCore.status()}", System.currentTimeMillis() < deadline)
            Thread.sleep(100)
        }
        assertTrue(home.notes.values.any { it.contains("message 1") })
    }
}
