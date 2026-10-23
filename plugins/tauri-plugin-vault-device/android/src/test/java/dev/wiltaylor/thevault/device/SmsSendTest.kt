package dev.wiltaylor.thevault.device

import android.Manifest
import android.app.Activity
import android.app.PendingIntent
import android.content.Context
import android.provider.Telephony.Sms
import android.telephony.SmsManager
import androidx.test.core.app.ApplicationProvider
import java.util.concurrent.Executors
import java.util.concurrent.Future
import java.util.concurrent.TimeUnit
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
 * Outbox X6: `device::sms_send` waits for Android's sent broadcast of every part before it
 * answers, then reads the sent row's `_id` from the SMS provider as the message's `device_ref`;
 * a failing part answers its SmsManager result code, and no answer in time is an error.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class SmsSendTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val phone = FakePhone(context)
    private val sms get() = shadowOf(context.getSystemService(SmsManager::class.java))
    private val pool = Executors.newSingleThreadExecutor()

    @Before
    fun setUp() {
        shadowOf(context as android.app.Application).grantPermissions(Manifest.permission.SEND_SMS)
    }

    @After
    fun tearDown() {
        pool.shutdownNow()
    }

    private fun data(sentTimeoutMs: Long = 10_000, rowWaitMs: Long = 2_000) =
        AndroidDeviceData(context, phone.capture(), { false }, sentTimeoutMs, rowWaitMs)

    /** `sms_send` on a thread of its own, as the core calls it from the command worker. */
    private fun send(data: AndroidDeviceData, to: String, body: String): Future<JSONObject> = pool.submit<JSONObject> {
        JSONObject(data.call("sms_send", JSONObject().put("to", to).put("body", body).toString()))
    }

    /** Waits for [found] to see the send, as Android does before it reports. */
    private fun <T> waitFor(found: () -> T?): T {
        val until = System.currentTimeMillis() + 10_000
        while (System.currentTimeMillis() < until) {
            found()?.let { return it }
            Thread.sleep(10)
        }
        throw AssertionError("the SMS was never handed to SmsManager")
    }

    private fun sentIntent(): PendingIntent = waitFor { sms.lastSentTextMessageParams?.sentIntent }

    @Test
    fun aSentSmsAnswersTheRowAndroidWroteForIt() {
        // An earlier message with the same words to the same number is not this one.
        phone.sms(40, "+61400000002", "Bins out", Sms.MESSAGE_TYPE_SENT)
        val answer = send(data(), "0400 000 002", "Bins out")
        val sent = sentIntent()
        assertEquals("0400 000 002", sms.lastSentTextMessageParams.destinationAddress)
        assertEquals("Bins out", sms.lastSentTextMessageParams.text)
        // Not answered until Android reports the send.
        Thread.sleep(200)
        assertTrue(!answer.isDone)
        // Android writes the row as it reports: here a moment after.
        phone.sms(41, "+61 400 000 002", "Bins in", Sms.MESSAGE_TYPE_SENT)
        sent.send(context, Activity.RESULT_OK, null)
        Thread.sleep(200)
        phone.sms(42, "0400000002", "Bins out", Sms.MESSAGE_TYPE_SENT)
        val ok = answer.get(5, TimeUnit.SECONDS).getJSONObject("ok")
        assertEquals("sms:42", ok.getString("device_ref"))
    }

    @Test
    fun aRowThatNeverAppearsAnswersNoDeviceRef() {
        val answer = send(data(rowWaitMs = 200), "+61400000002", "Bins out")
        sentIntent().send(context, Activity.RESULT_OK, null)
        val ok = answer.get(5, TimeUnit.SECONDS).getJSONObject("ok")
        assertTrue(ok.toString(), ok.isNull("device_ref"))
    }

    @Test
    fun aFailedSendAnswersItsResultCode() {
        val answer = send(data(), "+61400000002", "Bins out")
        sentIntent().send(context, SmsManager.RESULT_ERROR_NO_SERVICE, null)
        val err = answer.get(5, TimeUnit.SECONDS).getString("err")
        assertEquals("SmsManager result code 4 (no service)", err)
    }

    @Test
    fun noAnswerInTimeIsAnError() {
        val answer = send(data(sentTimeoutMs = 300), "+61400000002", "Bins out")
        sentIntent()
        val err = answer.get(5, TimeUnit.SECONDS).getString("err")
        assertTrue(err, err.startsWith("Android did not say within 0 s whether the SMS went."))
    }
}
