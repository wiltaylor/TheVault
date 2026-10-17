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

    private companion object {
        val PHOTO = byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 1, 2, 3)
        val THUMBNAIL = byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 7)
    }

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
        assertEquals("[\"grace@example.com\"]", grace.getJSONArray("emails").toString())
        assertEquals("[]", grace.getJSONArray("addresses").toString())
        assertEquals(Payloads.time(2_000), grace.getString("updated"))

        assertEquals(listOf("3", "4"), capture.contacts.changedSince(0).map { it.data.getString("contact_id") })

        val deleted = capture.contacts.deletedSince(0)
        assertEquals("contact_deleted", deleted.single().event)
        assertEquals(JSONObject().put("contact_id", "8").toString(), deleted.single().data.toString())
        assertEquals(2_000L, capture.contacts.latestChange())
        assertEquals(3_000L, capture.contacts.latestDeletion())
    }

    /** Contact 4 with one of every detail kind, as the contacts app holds them. */
    private fun everyDetail(full: ByteArray? = PHOTO, thumbnail: ByteArray? = THUMBNAIL) {
        phone.contact(4, "Grace Hopper", 2_000)
        phone.data(4, FakePhone.PHONE, "data1" to "0412 345 678", "data2" to 2, "data4" to "+61412345678")
        phone.data(4, FakePhone.PHONE, "data1" to "12345", "data2" to 3)
        phone.data(4, FakePhone.PHONE, "data1" to "+1 415 555 2671", "data2" to 0, "data3" to "Navy line")
        phone.data(4, FakePhone.PHONE, "data1" to "(02) 9999 0000", "data2" to 4)
        phone.data(4, FakePhone.EMAIL, "data1" to "grace@example.com", "data2" to 2)
        phone.data(4, FakePhone.EMAIL, "data1" to "amazing@example.org", "data2" to 0, "data3" to "Fan mail")
        phone.data(4, FakePhone.POSTAL, "data1" to "1 Navy Way\nArlington VA 22202\nUSA", "data2" to 2)
        phone.data(4, FakePhone.POSTAL, "data1" to "2 Home St\r\n\r\nNew York", "data2" to 1)
        phone.data(4, FakePhone.ORGANIZATION, "data1" to "US Navy", "data4" to "Rear Admiral", "data2" to 1)
        phone.data(4, FakePhone.NICKNAME, "data1" to "Amazing Grace")
        phone.data(4, FakePhone.EVENT, "data1" to "--12-09", "data2" to 3)
        phone.data(4, FakePhone.EVENT, "data1" to "1930-06-15", "data2" to 1)
        phone.data(4, FakePhone.WEBSITE, "data1" to "grace.example.com", "data2" to 7)
        phone.data(4, FakePhone.WEBSITE, "data1" to "https://cobol.example.org/", "data2" to 7)
        phone.data(4, FakePhone.NOTE, "data1" to "Found the first bug.")
        phone.photo(4, full, thumbnail)
    }

    private fun strings(a: org.json.JSONArray): List<String> = (0 until a.length()).map { a.getString(it) }

    @Test
    fun contactsReaderReadsEveryDetailKindWithLabels() {
        everyDetail()

        val grace = capture.contacts.changedSince(1_000).single().data
        assertEquals(listOf("+61412345678", "+14155552671", "+61299990000"), strings(grace.getJSONArray("phones")))
        assertEquals("the unnormalisable number's label drops with it", listOf("mobile", "Navy line", "other"), strings(grace.getJSONArray("phone_labels")))
        assertEquals(listOf("grace@example.com", "amazing@example.org"), strings(grace.getJSONArray("emails")))
        assertEquals(listOf("work", "Fan mail"), strings(grace.getJSONArray("email_labels")))
        assertEquals(listOf("1 Navy Way, Arlington VA 22202, USA", "2 Home St, New York"), strings(grace.getJSONArray("addresses")))
        assertEquals(listOf("work", "home"), strings(grace.getJSONArray("address_labels")))
        assertEquals("US Navy", grace.getString("organisation"))
        assertEquals("Rear Admiral", grace.getString("job_title"))
        assertEquals("Amazing Grace", grace.getString("nickname"))
        assertEquals("a birthday with no year, and not the anniversary", "--12-09", grace.getString("birthday"))
        assertEquals(listOf("grace.example.com", "https://cobol.example.org/"), strings(grace.getJSONArray("websites")))
        assertEquals("Found the first bug.", grace.getString("contact_note"))
        assertTrue(grace.isNull("person"))
        assertTrue(grace.isNull("synced"))

        // The full-size photo, not the thumbnail, stored as an asset before the payload is made.
        val photo = grace.getJSONObject("photo")
        assertEquals("01ASSET1", photo.getString("asset"))
        assertEquals(ContactPhotos.sha256(PHOTO), photo.getString("hash"))
        assertEquals("contact-4-photo.jpg", phone.assets.stored.single().first)
        assertArrayEquals(PHOTO, phone.assets.stored.single().second)

        // Read again, and by a capture made afresh, the same bytes reuse the asset.
        assertEquals(photo.toString(), capture.contacts.get("4").getJSONObject("photo").toString())
        assertEquals(photo.toString(), phone.capture().contacts.get("4").getJSONObject("photo").toString())
        assertEquals("stored once", 1, phone.assets.stored.size)

        // A new photo is a new asset.
        phone.photo(4, byteArrayOf(9, 9, 9))
        assertEquals("01ASSET2", capture.contacts.get("4").getJSONObject("photo").getString("asset"))
        assertEquals(2, phone.assets.stored.size)
    }

    @Test
    fun aContactWithOnlyAThumbnailOrNoDetailsReadsAsEmpty() {
        everyDetail(full = null)
        assertEquals(ContactPhotos.sha256(THUMBNAIL), capture.contacts.get("4").getJSONObject("photo").getString("hash"))
        assertArrayEquals(THUMBNAIL, phone.assets.stored.single().second)

        val ada = capture.contacts.get("3")
        for (key in listOf("addresses", "address_labels", "websites")) assertEquals(key, "[]", ada.getJSONArray(key).toString())
        assertEquals(listOf("mobile"), strings(ada.getJSONArray("phone_labels")))
        assertEquals(listOf("other"), strings(ada.getJSONArray("email_labels")))
        for (key in listOf("organisation", "job_title", "nickname", "birthday", "contact_note")) assertEquals(key, "", ada.getString(key))
        assertTrue(ada.isNull("photo"))
    }

    @Test
    fun birthdaysAreFullDatesOrMonthAndDay() {
        assertEquals("1906-12-09", ContactsReader.birthday("1906-12-09"))
        assertEquals("1906-12-09", ContactsReader.birthday("19061209"))
        assertEquals("1906-12-09", ContactsReader.birthday("1906-12-09T00:00:00.000Z"))
        assertEquals("--12-09", ContactsReader.birthday("--12-09"))
        assertEquals("--12-09", ContactsReader.birthday("--1209"))
        assertEquals(null, ContactsReader.birthday("December"))
        assertEquals(null, ContactsReader.birthday(""))
    }

    @Test
    fun anEditToAnyDetailIsAChange() {
        everyDetail()
        val edits = listOf(
            FakePhone.POSTAL to arrayOf<Pair<String, Any?>>("data1" to "3 New Rd", "data2" to 1),
            FakePhone.ORGANIZATION to arrayOf<Pair<String, Any?>>("data1" to "Remington Rand", "data4" to "Engineer"),
            FakePhone.NICKNAME to arrayOf<Pair<String, Any?>>("data1" to "Grandma COBOL"),
            FakePhone.EVENT to arrayOf<Pair<String, Any?>>("data1" to "1906-12-09", "data2" to 3),
            FakePhone.WEBSITE to arrayOf<Pair<String, Any?>>("data1" to "https://hopper.example.com"),
            FakePhone.NOTE to arrayOf<Pair<String, Any?>>("data1" to "Nanoseconds."),
            FakePhone.PHONE to arrayOf<Pair<String, Any?>>("data1" to "0499 999 999", "data2" to 0, "data3" to "Lab"),
            FakePhone.EMAIL to arrayOf<Pair<String, Any?>>("data1" to "hopper@example.com", "data2" to 1),
        )
        val seen = mutableListOf<String>()
        for ((mime, values) in edits) {
            val before = capture.contacts.latestChange()
            phone.edit(4, mime, *values)
            assertTrue("$mime moves the last change", capture.contacts.latestChange() > before)
            val changed = capture.contacts.changedSince(before)
            assertEquals(mime, listOf("4"), changed.map { it.data.getString("contact_id") })
            seen += changed.single().data.toString()
        }
        val last = capture.contacts.get("4")
        assertEquals(listOf("3 New Rd"), strings(last.getJSONArray("addresses")))
        assertEquals("Remington Rand", last.getString("organisation"))
        assertEquals("1906-12-09", last.getString("birthday"))
        assertEquals(listOf("Lab"), strings(last.getJSONArray("phone_labels")))
        assertEquals(listOf("home"), strings(last.getJSONArray("email_labels")))

        // The photo too, written as another app writes it.
        val before = capture.contacts.latestChange()
        context().contentResolver.openAssetFileDescriptor(
            android.net.Uri.parse("content://com.android.contacts/raw_contacts/4/display_photo"),
            "rw",
        )!!.use { it.createOutputStream().use { out -> out.write(byteArrayOf(5, 5)) } }
        assertEquals(listOf("4"), capture.contacts.changedSince(before).map { it.data.getString("contact_id") })
        assertEquals(ContactPhotos.sha256(byteArrayOf(5, 5)), capture.contacts.get("4").getJSONObject("photo").getString("hash"))
    }

    private fun context(): android.content.Context = ApplicationProvider.getApplicationContext()

    @Test
    fun upsertWritesEveryDetailKindAndReadsBackTheSame() {
        everyDetail()
        val photo = phone.assets.put("face.png", byteArrayOf(0x89.toByte(), 'P'.code.toByte(), 'N'.code.toByte(), 'G'.code.toByte(), 1))
            .getString("asset")
        val details = JSONObject()
            .put("name", "Grace Brewster Hopper")
            .put("phones", org.json.JSONArray(listOf("+61499999999", "+14155552671")))
            .put("phone_labels", org.json.JSONArray(listOf("work", "Navy line")))
            .put("emails", org.json.JSONArray(listOf("grace@navy.example.mil")))
            .put("email_labels", org.json.JSONArray(listOf("Duty")))
            .put("addresses", org.json.JSONArray(listOf("1 Navy Way, Arlington VA 22202, USA", "9 Main St, Cambridge MA")))
            .put("address_labels", org.json.JSONArray(listOf("work", "other")))
            .put("organisation", "Harvard")
            .put("job_title", "Research Fellow")
            .put("nickname", "Amazing Grace")
            .put("birthday", "--12-09")
            .put("websites", org.json.JSONArray(listOf("https://grace.example.com")))
            .put("contact_note", "Wrote the first compiler.")
            .put("photo", photo)
        val data = AndroidDeviceData(context(), capture)
        fun upsert(contact: JSONObject): JSONObject =
            JSONObject(data.call("contacts_upsert", JSONObject().put("contact", contact).toString())).getJSONObject("ok")

        // Onto the existing contact, and onto a new one.
        assertEquals("4", upsert(JSONObject(details.toString()).put("contact_id", "4")).getString("contact_id"))
        val added = upsert(JSONObject(details.toString()).put("contact_id", JSONObject.NULL)).getString("contact_id")
        for (id in listOf("4", added)) {
            val read = capture.contacts.get(id)
            for (key in details.keys()) {
                if (key == "photo") continue
                assertEquals("$id $key", details.get(key).toString(), read.get(key).toString())
            }
            assertEquals("the written photo reads back as its own asset", photo, read.getJSONObject("photo").getString("asset"))
            assertEquals(ContactPhotos.sha256(phone.assets.stored[0].second), read.getJSONObject("photo").getString("hash"))
        }
        assertEquals("no photo stored again", 1, phone.assets.stored.size)
        // Labels go back as types, and a custom one as its text.
        assertEquals(listOf("+61499999999 3", "+14155552671 0 Navy line"), phone.rows(4, FakePhone.PHONE, "data1", "data2", "data3"))
        assertEquals(listOf("--12-09 3", "1930-06-15 1"), phone.rows(4, FakePhone.EVENT, "data1", "data2").sorted())
        assertArrayEquals(phone.assets.stored[0].second, phone.photoOf(4))

        // A key left out leaves its detail alone; a label list left out keeps each value's label;
        // a null photo clears it.
        upsert(
            JSONObject()
                .put("contact_id", "4")
                .put("phones", org.json.JSONArray(listOf("+14155552671", "+61412345678")))
                .put("nickname", JSONObject.NULL)
                .put("photo", JSONObject.NULL),
        )
        val after = capture.contacts.get("4")
        assertEquals(listOf("Navy line", "mobile"), strings(after.getJSONArray("phone_labels")))
        assertEquals("", after.getString("nickname"))
        assertTrue(after.isNull("photo"))
        assertEquals("Grace Brewster Hopper", after.getString("name"))
        assertEquals("Harvard", after.getString("organisation"))
        assertEquals(listOf("grace@navy.example.mil"), strings(after.getJSONArray("emails")))
        assertEquals("Wrote the first compiler.", after.getString("contact_note"))
    }

    @Test
    fun upsertWithAPersonRecordsEveryDetailAsAgreed() {
        everyDetail()
        val photo = phone.assets.put("face.jpg", PHOTO).getString("asset")
        AndroidDeviceData(context(), capture).call(
            "contacts_upsert",
            JSONObject().put(
                "contact",
                JSONObject()
                    .put("contact_id", "4")
                    .put("person", "01PERSONGRACE0000000000000")
                    .put("name", "Grace Hopper")
                    .put("emails", org.json.JSONArray(listOf("grace@example.com")))
                    .put("birthday", "--12-09")
                    .put("photo", photo),
            ).toString(),
        )
        val row = phone.personRow(4)!!
        assertEquals("01PERSONGRACE0000000000000", row[0])
        assertEquals(ContactsReader.SYNCED_FORMAT, row[2])
        val synced = capture.contacts.get("4").getJSONObject("synced")
        assertEquals("Grace Hopper", synced.getString("name"))
        assertEquals("[\"grace@example.com\"]", synced.getJSONArray("emails").toString())
        assertEquals("[\"work\"]", synced.getJSONArray("email_labels").toString())
        assertEquals("--12-09", synced.getString("birthday"))
        assertEquals(JSONObject().put("asset", photo).put("hash", ContactPhotos.sha256(PHOTO)).toString(), synced.getJSONObject("photo").toString())
        assertTrue("keys not written are not agreed", !synced.has("phones") && !synced.has("addresses"))
        assertEquals("the photo was the same already: it reuses the asset", photo, capture.contacts.get("4").getJSONObject("photo").getString("asset"))
        assertEquals(1, phone.assets.stored.size)
    }

    @Test
    fun syncedStateFromAnOlderVersionHasItsEmailsMigrated() {
        val old = JSONObject()
            .put("name", "Ada Lovelace")
            .put("phones", org.json.JSONArray(listOf("+61412345678")))
            .put("addresses", org.json.JSONArray(listOf("ada@example.com")))
        phone.data(3, ContactsReader.PERSON_MIME, "data1" to "01PERSONADA000000000000000", "data2" to old.toString())

        // Before the rewrite, the reader already reads the old form as emails.
        val read = capture.contacts.get("3").getJSONObject("synced")
        assertEquals("[\"ada@example.com\"]", read.getJSONArray("emails").toString())
        assertTrue(!read.has("addresses"))

        // Capture rewrites it once, before it reads contacts.
        capture.begin()
        capture.captureContacts()
        val row = phone.personRow(3)!!
        assertEquals("01PERSONADA000000000000000", row[0])
        assertEquals(ContactsReader.SYNCED_FORMAT, row[2])
        val stored = JSONObject(row[1]!!)
        assertEquals("Ada Lovelace", stored.getString("name"))
        assertEquals("[\"+61412345678\"]", stored.getJSONArray("phones").toString())
        assertEquals("[\"ada@example.com\"]", stored.getJSONArray("emails").toString())
        assertTrue(!stored.has("addresses"))
        assertEquals("nothing is left to rewrite", 0, capture.contacts.migrateSynced())

        // A row in this version's form keeps its postal addresses.
        val now = JSONObject().put("emails", org.json.JSONArray()).put("addresses", org.json.JSONArray(listOf("1 Navy Way")))
        phone.contact(4, "Grace Hopper", 2_000)
        phone.data(4, ContactsReader.PERSON_MIME, "data1" to "01PERSONGRACE0000000000000", "data2" to now.toString(), "data3" to ContactsReader.SYNCED_FORMAT)
        assertEquals(0, capture.contacts.migrateSynced())
        assertEquals(now.toString(), capture.contacts.get("4").getJSONObject("synced").toString())
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
