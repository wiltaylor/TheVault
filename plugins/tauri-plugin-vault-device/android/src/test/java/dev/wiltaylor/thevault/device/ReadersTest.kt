package dev.wiltaylor.thevault.device

import android.provider.CallLog.Calls
import android.provider.Telephony.Sms
import androidx.test.core.app.ApplicationProvider
import org.json.JSONObject
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.time.ZoneOffset

/** S5: the provider readers return the right rows from fake providers, as device event payloads. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class ReadersTest {
    private lateinit var phone: FakePhone
    private lateinit var capture: DeviceCapture

    @Before
    fun setUp() {
        phone = FakePhone(ApplicationProvider.getApplicationContext())
        capture = phone.capture()
        phone.contact(3, "Ada Lovelace", 1_000, listOf("0412 345 678" to "+61412345678"), listOf("ada@example.com"))
    }

    @Test
    fun numbersNormaliseToE164WithThePhonesRegion() {
        val au = PhoneNumbers("au")
        assertEquals("+61412345678", au.normalise("0412 345 678"))
        assertEquals("+61412345678", au.normalise("+61 412-345-678"))
        assertEquals("+14155552671", au.normalise("+1 415 555 2671"))
        assertEquals(null, au.normalise("12345"))
        assertEquals(null, au.normalise("Telstra"))
        assertEquals(null, au.normalise(""))
    }

    @Test
    fun payloadTimesAreRfc3339WithAnOffset() {
        assertEquals("2026-10-20T09:30:00+11:00", Payloads.time(1_792_449_000_000, ZoneOffset.ofHours(11)))
        assertEquals(1_792_449_000_000, Payloads.parseTime("2026-10-20T09:30:00+11:00"))
    }

    @Test
    fun smsReaderReturnsReceivedAndSentSmsAndLeavesOutDrafts() {
        phone.sms(1, "+61412345678", "hello", Sms.MESSAGE_TYPE_INBOX)
        phone.sms(2, "0412345678", "hi back", Sms.MESSAGE_TYPE_SENT)
        phone.sms(3, "0412345678", "unsent", Sms.MESSAGE_TYPE_DRAFT)
        phone.sms(4, "GOV", "your code is 1234", Sms.MESSAGE_TYPE_INBOX)

        val rows = capture.sms.since(0)
        assertEquals(listOf("sms_received", "sms_sent", "sms_received"), rows.map { it.event })
        assertEquals(listOf(1L, 2L, 4L), rows.map { it.key })

        val received = rows[0].data
        assertEquals("sms:1", received.getString("device_ref"))
        assertEquals("7", received.getString("thread_ref"))
        assertEquals("+61412345678", received.getString("number"))
        assertEquals("+61412345678", received.getString("number_raw"))
        assertEquals("hello", received.getString("body"))
        assertEquals(Payloads.time(60_000 - 5_000), received.getString("sent"))
        assertEquals("3", received.getJSONObject("contact").getString("id"))
        assertEquals("Ada Lovelace", received.getJSONObject("contact").getString("name"))

        assertEquals("+61412345678", rows[1].data.getString("number"))
        assertEquals(Payloads.time(120_000), rows[1].data.getString("sent"))

        val shortCode = rows[2].data
        assertTrue(shortCode.isNull("number"))
        assertEquals("GOV", shortCode.getString("number_raw"))
        assertTrue(shortCode.isNull("contact"))

        assertEquals(listOf(4L), capture.sms.since(2).map { it.key })
        assertEquals(4L, capture.sms.latestKey())
    }

    @Test
    fun mmsReaderJoinsTextPartsStoresTheRestAndListsOtherRecipients() {
        val image = byteArrayOf(1, 2, 3, 4)
        phone.mms(5, "0412345678", listOf(FakePhone.SELF, "+61499999999", "0412345678"), subject = "Trip")
        phone.part(10, 5, "application/smil", text = "<smil/>")
        phone.part(11, 5, "text/plain", text = "look at this")
        phone.part(12, 5, "image/jpeg", name = "beach.jpg", bytes = image)
        phone.part(13, 5, "text/plain", text = "and this")
        phone.mms(6, "0412345678", emptyList())
        phone.db.execSQL("UPDATE mms SET msg_box = 2 WHERE _id = 6")

        val rows = capture.mms.since(0)
        assertEquals(1, rows.size)
        val mms = rows[0]
        assertEquals("mms_received", mms.event)
        assertEquals(5L, mms.key)
        val data = mms.data
        assertEquals("mms:5", data.getString("device_ref"))
        assertEquals("9", data.getString("thread_ref"))
        assertEquals("+61412345678", data.getString("number"))
        assertEquals("Ada Lovelace", data.getJSONObject("contact").getString("name"))
        assertEquals("Trip", data.getString("subject"))
        assertEquals("look at this\nand this", data.getString("body"))
        assertEquals(Payloads.time(300_000), data.getString("sent"))

        val to = data.getJSONArray("to")
        assertEquals("only the other recipient: not this phone, not the sender", 1, to.length())
        assertEquals("+61499999999", to.getJSONObject(0).getString("number"))
        assertTrue(to.getJSONObject(0).isNull("contact"))

        val attachments = data.getJSONArray("attachments")
        assertEquals(1, attachments.length())
        val a = attachments.getJSONObject(0)
        assertEquals("01ASSET1", a.getString("asset"))
        assertEquals("beach.jpg", a.getString("filename"))
        assertEquals("image/jpeg", a.getString("mime"))
        assertEquals(4, a.getLong("size"))
        assertEquals("beach.jpg", phone.assets.stored.single().first)
        assertArrayEquals(image, phone.assets.stored.single().second)
    }

    @Test
    fun mmsPartsWithoutAnExtensionAreStoredWithTheOneOfTheirContentType() {
        phone.mms(7, "0412345678", emptyList())
        phone.part(20, 7, "image/jpeg", name = "image000001", bytes = byteArrayOf(1))
        phone.part(21, 7, "image/png", bytes = byteArrayOf(2))
        phone.part(22, 7, "image/jpeg", name = "IMG_1.JPEG", bytes = byteArrayOf(3))
        phone.part(23, 7, "text/x-vcard", name = "contact", bytes = byteArrayOf(4))
        phone.part(24, 7, "application/octet-stream", name = "blob", bytes = byteArrayOf(5))

        val attachments = capture.mms.since(0).single().data.getJSONArray("attachments")
        val names = listOf("image000001.jpg", "part-21.png", "IMG_1.JPEG", "contact.vcf", "blob")
        assertEquals(names, phone.assets.stored.map { it.first })
        assertEquals(names, (0 until attachments.length()).map { attachments.getJSONObject(it).getString("filename") })
        assertEquals("image/jpeg", attachments.getJSONObject(0).getString("mime"))
    }

    @Test
    fun fileNamesTakeTheExtensionOfTheirContentType() {
        assertEquals("image000001.jpg", FileNames.withExtension("image000001", "image/jpeg"))
        assertEquals("photo.dat.png", FileNames.withExtension("photo.dat", "image/png"))
        assertEquals("photo.png", FileNames.withExtension("photo.png", "IMAGE/PNG; name=photo.png"))
        assertEquals("blob", FileNames.withExtension("blob", "application/x-unknown-thing"))
    }

    @Test
    fun callLogReaderMapsDirectionsAndLeavesOutVoicemail() {
        phone.call(1, "0412345678", Calls.INCOMING_TYPE, 65)
        phone.call(2, "+61499999999", Calls.OUTGOING_TYPE, 30)
        phone.call(3, "0412345678", Calls.MISSED_TYPE, 12)
        phone.call(4, "0412345678", Calls.VOICEMAIL_TYPE, 20)
        phone.call(5, "0412345678", Calls.REJECTED_TYPE, 0)

        val rows = capture.calls.since(0)
        assertEquals(listOf(1L, 2L, 3L, 5L), rows.map { it.key })
        assertEquals(listOf("in", "out", "missed", "missed"), rows.map { it.data.getString("direction") })
        assertEquals(listOf(65L, 30L, 0L, 0L), rows.map { it.data.getLong("duration") })
        val first = rows[0].data
        assertEquals("call_ended", rows[0].event)
        assertEquals("call:1", first.getString("device_ref"))
        assertEquals("+61412345678", first.getString("number"))
        assertEquals("0412345678", first.getString("number_raw"))
        assertEquals("3", first.getJSONObject("contact").getString("id"))
        assertEquals(Payloads.time(60_000), first.getString("started"))
    }

    @Test
    fun contactsReaderReturnsChangesWithE164PhonesAndDeletions() {
        phone.contact(4, "Grace Hopper", 2_000, listOf("+1 415 555 2671" to "+14155552671", "12345" to ""), listOf("grace@example.com"))
        phone.deleteContact(8, 3_000)

        val changed = capture.contacts.changedSince(1_000)
        assertEquals(1, changed.size)
        val grace = changed[0].data
        assertEquals("contact_changed", changed[0].event)
        assertEquals(2_000L, changed[0].key)
        assertEquals("4", grace.getString("contact_id"))
        assertEquals("Grace Hopper", grace.getString("name"))
        assertEquals("[\"+14155552671\"]", grace.getJSONArray("phones").toString())
        assertEquals("[\"grace@example.com\"]", grace.getJSONArray("addresses").toString())
        assertEquals(Payloads.time(2_000), grace.getString("updated"))

        assertEquals(listOf("3", "4"), capture.contacts.changedSince(0).map { it.data.getString("contact_id") })

        val deleted = capture.contacts.deletedSince(0)
        assertEquals("contact_deleted", deleted.single().event)
        assertEquals(JSONObject().put("contact_id", "8").toString(), deleted.single().data.toString())
        assertEquals(2_000L, capture.contacts.latestChange())
        assertEquals(3_000L, capture.contacts.latestDeletion())
    }

    @Test
    fun deviceModuleReadsFilterAndLimit() {
        phone.sms(1, "+61412345678", "one", Sms.MESSAGE_TYPE_INBOX, thread = 1)
        phone.sms(2, "+61499999999", "two", Sms.MESSAGE_TYPE_INBOX, thread = 2)
        phone.sms(3, "0412345678", "three", Sms.MESSAGE_TYPE_SENT, thread = 1)
        phone.call(1, "0412345678", Calls.INCOMING_TYPE, 5)
        phone.call(2, "+61499999999", Calls.INCOMING_TYPE, 5)

        val data = AndroidDeviceData(ApplicationProvider.getApplicationContext(), capture)
        fun ok(method: String, args: String): Any = JSONObject(data.call(method, args)).get("ok")

        val byNumber = ok("sms_list", """{"filter":{"number":"+61412345678"}}""") as org.json.JSONArray
        assertEquals(listOf("sms:3", "sms:1"), (0 until byNumber.length()).map { byNumber.getJSONObject(it).getString("device_ref") })
        val byThread = ok("sms_list", """{"filter":{"thread_ref":"2","limit":5}}""") as org.json.JSONArray
        assertEquals("two", byThread.getJSONObject(0).getString("body"))
        val limited = ok("sms_list", """{"filter":{"limit":1}}""") as org.json.JSONArray
        assertEquals(1, limited.length())
        assertEquals("one", (ok("sms_get", """{"device_ref":"sms:1"}""") as JSONObject).getString("body"))
        assertEquals(1, (ok("calls_list", """{"filter":{"number":"+61499999999"}}""") as org.json.JSONArray).length())
        val contacts = ok("contacts_list", """{"filter":{"name":"Ada Lovelace"}}""") as org.json.JSONArray
        assertEquals("3", contacts.getJSONObject(0).getString("contact_id"))
        assertEquals("Ada Lovelace", (ok("contacts_get", """{"contact_id":"3"}""") as JSONObject).getString("name"))

        assertTrue(JSONObject(data.call("sms_get", """{"device_ref":"sms:99"}""")).has("err"))
        assertTrue(JSONObject(data.call("no_such_thing", "{}")).getString("err").contains("no_such_thing"))
    }
}
