package dev.wiltaylor.thevault.device

import android.content.Context
import android.content.Intent
import android.os.Looper
import android.provider.CallLog.Calls
import android.provider.ContactsContract
import android.provider.Telephony
import android.provider.Telephony.Sms
import android.telephony.TelephonyManager
import androidx.test.core.app.ApplicationProvider
import androidx.work.ListenableWorker
import androidx.work.WorkManager
import androidx.work.testing.TestListenableWorkerBuilder
import androidx.work.testing.WorkManagerTestInitHelper
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
 * S5: receivers, the Telephony and contacts observers and the content-URI job enqueue
 * `sms_received`, `mms_received`, `call_ended` and `contact_changed` with normalised numbers; the
 * catch-up skips rows already captured; the first-run import reads only its date range.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class CaptureTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private lateinit var phone: FakePhone
    private lateinit var capture: DeviceCapture

    @Before
    fun setUp() {
        WorkManagerTestInitHelper.initializeTestWorkManager(context)
        phone = FakePhone(context)
        phone.contact(3, "Ada Lovelace", 1_000, listOf("0412 345 678" to "+61412345678"))
        capture = phone.install()
        capture.begin()
    }

    @After
    fun tearDown() {
        CaptureObservers.reset()
        Device.reset()
    }

    private fun sink() = phone.sink

    @Test
    fun theReceiversAreDeclaredForTheirBroadcasts() {
        val pm = context.packageManager
        fun receivers(action: String) = pm.queryBroadcastReceivers(Intent(action).setPackage(context.packageName), 0)
            .map { it.activityInfo.name }
        assertEquals(listOf(SmsReceivedReceiver::class.java.name), receivers(Telephony.Sms.Intents.SMS_RECEIVED_ACTION))
        assertEquals(listOf(PhoneStateReceiver::class.java.name), receivers(TelephonyManager.ACTION_PHONE_STATE_CHANGED))
    }

    @Test
    fun smsReceivedEnqueuesTheNewSmsWithItsNormalisedNumber() {
        phone.sms(1, "0412 345 678", "are you there?", Sms.MESSAGE_TYPE_INBOX)

        SmsReceivedReceiver().onReceive(context, Intent(Telephony.Sms.Intents.SMS_RECEIVED_ACTION))

        val e = sink().events.single()
        assertEquals("sms_received", e.event)
        assertEquals("sms:1", e.data.getString("device_ref"))
        assertEquals("+61412345678", e.data.getString("number"))
        assertEquals("0412 345 678", e.data.getString("number_raw"))
        assertEquals("Ada Lovelace", e.data.getJSONObject("contact").getString("name"))
        assertEquals(1, sink().flushes)
    }

    @Test
    fun smsReceivedWithNoRowYetEnqueuesNothing() {
        SmsReceivedReceiver().onReceive(context, Intent(Telephony.Sms.Intents.SMS_RECEIVED_ACTION))
        assertTrue(sink().events.isEmpty())
        assertEquals(0, sink().flushes)
    }

    @Test
    fun theEndOfACallEnqueuesCallEndedFromTheCallLog() {
        phone.call(1, "+61412345678", Calls.INCOMING_TYPE, 42)

        PhoneStateReceiver().onReceive(
            context,
            Intent(TelephonyManager.ACTION_PHONE_STATE_CHANGED).putExtra(TelephonyManager.EXTRA_STATE, TelephonyManager.EXTRA_STATE_RINGING),
        )
        assertTrue("ringing is not the end of a call", sink().events.isEmpty())

        PhoneStateReceiver().onReceive(
            context,
            Intent(TelephonyManager.ACTION_PHONE_STATE_CHANGED).putExtra(TelephonyManager.EXTRA_STATE, TelephonyManager.EXTRA_STATE_IDLE),
        )
        val e = sink().events.single()
        assertEquals("call_ended", e.event)
        assertEquals("call:1", e.data.getString("device_ref"))
        assertEquals("in", e.data.getString("direction"))
        assertEquals(42L, e.data.getLong("duration"))
        assertEquals("+61412345678", e.data.getString("number"))
    }

    @Test
    fun anMmsIsCapturedByObservingTheTelephonyProvider() {
        CaptureObservers.register(context)
        phone.mms(4, "+61412345678", listOf(FakePhone.SELF))
        phone.part(20, 4, "text/plain", text = "photo attached")
        phone.part(21, 4, "image/png", name = "a.png", bytes = byteArrayOf(9))

        phone.notify(Telephony.Mms.CONTENT_URI)
        shadowOf(Looper.getMainLooper()).idle()

        val e = sink().events.single()
        assertEquals("mms_received", e.event)
        assertEquals("mms:4", e.data.getString("device_ref"))
        assertEquals("+61412345678", e.data.getString("number"))
        assertEquals("photo attached", e.data.getString("body"))
        assertEquals("a.png", e.data.getJSONArray("attachments").getJSONObject(0).getString("filename"))
        assertEquals(0, e.data.getJSONArray("to").length())
        assertEquals(1, sink().flushes)
    }

    @Test
    fun aContactEditIsCapturedByTheContactsObserver() {
        CaptureObservers.register(context)
        phone.contact(3, "Ada King", 5_000, listOf("0412 345 678" to "+61412345678"), listOf("ada@example.com"))

        phone.notify(ContactsContract.Contacts.CONTENT_URI)
        shadowOf(Looper.getMainLooper()).idle()

        val e = sink().events.single()
        assertEquals("contact_changed", e.event)
        assertEquals("3", e.data.getString("contact_id"))
        assertEquals("Ada King", e.data.getString("name"))
        assertEquals("[\"+61412345678\"]", e.data.getJSONArray("phones").toString())
    }

    @Test
    fun theContentUriJobCapturesContactChangesAndWatchesAgain() {
        phone.contact(9, "Alan Turing", 6_000, listOf("+44 20 7946 0018" to "+442079460018"))
        phone.deleteContact(3, 7_000)

        val result = TestListenableWorkerBuilder<ProviderChangeWorker>(context).build().doWork()

        assertEquals(ListenableWorker.Result.success(), result)
        assertEquals(listOf("contact_changed", "contact_deleted"), sink().events.map { it.event })
        assertEquals("+442079460018", sink().events[0].data.getJSONArray("phones").getString(0))
        assertEquals("3", sink().events[1].data.getString("contact_id"))
        val watching = WorkManager.getInstance(context).getWorkInfosForUniqueWork("vault-device-providers").get()
        assertTrue("the job queues itself again", watching.isNotEmpty())
    }

    @Test
    fun catchUpSkipsRowsAlreadyCaptured() {
        phone.sms(1, "+61412345678", "live", Sms.MESSAGE_TYPE_INBOX)
        SmsReceivedReceiver().onReceive(context, Intent(Telephony.Sms.Intents.SMS_RECEIVED_ACTION))
        phone.call(1, "+61412345678", Calls.OUTGOING_TYPE, 3)
        PhoneStateReceiver().onReceive(
            context,
            Intent(TelephonyManager.ACTION_PHONE_STATE_CHANGED).putExtra(TelephonyManager.EXTRA_STATE, TelephonyManager.EXTRA_STATE_IDLE),
        )
        assertEquals(listOf("sms:1", "call:1"), sink().refs())

        // Missed while the app was stopped.
        phone.sms(2, "+61499999999", "missed", Sms.MESSAGE_TYPE_INBOX)
        phone.sms(3, "+61412345678", "sent", Sms.MESSAGE_TYPE_SENT)
        phone.call(2, "+61499999999", Calls.MISSED_TYPE, 0)
        phone.mms(1, "+61499999999", emptyList())

        val result = TestListenableWorkerBuilder<CatchUpWorker>(context).build().doWork()
        assertEquals(ListenableWorker.Result.success(), result)
        assertEquals(listOf("sms:1", "call:1", "sms:2", "sms:3", "mms:1", "call:2"), sink().refs())
        assertEquals("every row once", sink().refs().distinct(), sink().refs())

        assertEquals("a second catch-up finds nothing new", 0, capture.catchUp())
        assertEquals(6, sink().events.size)
    }

    @Test
    fun captureStartsAtTheNewestRowsSoHistoryIsNotCapturedUnlessImported() {
        val history = FakePhone(context)
        history.sms(1, "+61412345678", "last year", Sms.MESSAGE_TYPE_INBOX, date = 1_000)
        history.sms(2, "+61412345678", "last month", Sms.MESSAGE_TYPE_INBOX, date = 50_000)
        history.call(1, "+61412345678", Calls.INCOMING_TYPE, 9, date = 60_000)
        history.contact(5, "Old Friend", 55_000)
        val fresh = history.capture("fresh-cursors")

        assertEquals("nothing before capture began", 0, fresh.catchUp())
        history.sms(3, "+61412345678", "today", Sms.MESSAGE_TYPE_INBOX, date = 900_000)
        assertEquals(1, fresh.catchUp())
        assertEquals(listOf("sms:3"), history.sink.refs())

        // An import reads the range whatever the cursors say, and moves no cursor.
        val range = 40_000L..100_000L
        fun refs(kind: String) = fresh.history(kind, range, null).items.map {
            it.data.optString("device_ref", it.data.optString("contact_id"))
        }
        assertEquals(listOf("sms:2"), refs("sms"))
        assertEquals(listOf("call:1"), refs("calls"))
        assertEquals(listOf("5"), refs("contacts"))
        assertEquals("the import queued nothing itself", listOf("sms:3"), history.sink.refs())
        assertEquals(0, fresh.catchUp())
    }
}
