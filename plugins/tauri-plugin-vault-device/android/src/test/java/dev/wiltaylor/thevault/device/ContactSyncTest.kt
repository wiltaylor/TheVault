package dev.wiltaylor.thevault.device

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.work.Configuration
import androidx.work.testing.SynchronousExecutor
import androidx.work.testing.WorkManagerTestInitHelper
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * S5: two-way contact sync through `ContactsContract` (D16, D24). The contacts provider is the
 * fake one, the core and the default device scripts the real ones, through JNI. A contact change
 * makes or updates its person, and the contact takes the person's id in a data row of the app's
 * own; a person change, arriving as an `upsert_contact` command, rewrites the contact; deleting
 * the contact only unlinks the person.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class ContactSyncTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val home = FakeHome()
    private lateinit var phone: FakePhone
    private lateinit var capture: DeviceCapture

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
        phone.contact(3, "Ada Lovelace", 1_000, listOf("0412 345 678" to "+61412345678"), listOf("ada@example.com"))
        // Photos go into the local vault through the core, as in the app.
        capture = phone.install(assets = CoreAssets)
        assertTrue(Device.openCore(context))
        // The device may write contacts: the core takes that from the server with its commands.
        home.settings = JSONObject()
            .put("contacts_write", true)
            .put("commands", JSONArray().put("upsert_contact").put("delete_contact"))
        assertEquals("[]", VaultCore.runCommands())
    }

    @After
    fun tearDown() {
        Device.reset()
        home.close()
    }

    /** The phone's contact [id] has changed: capture it, as the observer does, and run the handlers. */
    private fun changed(id: Long) {
        VaultCore.enqueue("contact_changed", capture.contacts.get(id.toString()).toString(), System.currentTimeMillis())
        VaultCore.runPending()
    }

    /** The local vault's people, from the notes waiting to sync. */
    private fun people(): List<JSONObject> {
        val queued = JSONArray(VaultCore.queued())
        return (0 until queued.length()).map { queued.getJSONObject(it) }.filter { it.optString("type") == "person" }
    }

    /** The person's `device_contacts`, or null when it has none. */
    private fun links(person: JSONObject): JSONObject? {
        val blocks = person.getJSONArray("blocks")
        return (0 until blocks.length()).map { blocks.getJSONObject(it) }
            .firstOrNull { it.optString("kind") == "dict" && it.getJSONArray("labels").optString(0) == "device_contacts" }
            ?.getJSONObject("fields")
    }

    private fun person(): JSONObject = people().single()

    private fun vaultRow(contact: Long): String = phone.rows(contact, ContactsReader.PERSON_MIME).single()

    @Test
    fun aContactChangeMakesAPersonAndTheContactHoldsItsId() {
        changed(3)

        val ada = person()
        val fields = ada.getJSONObject("fields")
        assertEquals("Ada Lovelace", fields.getString("title"))
        assertEquals("[\"+61412345678\"]", fields.getJSONArray("phones").toString())
        assertEquals("3", links(ada)!!.getString("device_${FakeHome.DEVICE_ID}"))

        // The contact holds the person's id and what the two now agree on, and keeps its own
        // number formatting: the fields already agreed, so only the app's row was written.
        val row = vaultRow(3)
        assertTrue(row, row.startsWith(ada.getString("id") + " "))
        val synced = JSONObject(row.substringAfter(" "))
        assertEquals("Ada Lovelace", synced.getString("name"))
        assertEquals("[\"+61412345678\"]", synced.getJSONArray("phones").toString())
        assertEquals(listOf("0412 345 678"), phone.rows(3, FakePhone.PHONE))

        val read = capture.contacts.get("3")
        assertEquals(ada.getString("id"), read.getString("person"))
        assertEquals("Ada Lovelace", read.getJSONObject("synced").getString("name"))
    }

    @Test
    fun aContactChangeUpdatesThePerson() {
        changed(3)
        val id = person().getString("id")

        phone.contact(3, "Ada King", 5_000, listOf("0412 345 678" to "+61412345678", "0499 999 999" to "+61499999999"), listOf("ada@example.com"))
        changed(3)

        val ada = person()
        assertEquals(id, ada.getString("id"))
        assertEquals("Ada King", ada.getJSONObject("fields").getString("title"))
        assertEquals("[\"+61412345678\",\"+61499999999\"]", ada.getJSONObject("fields").getJSONArray("phones").toString())
        assertEquals("Ada King", JSONObject(vaultRow(3).substringAfter(" ")).getString("name"))
        assertEquals("one person", 1, people().size)
    }

    @Test
    fun aPersonChangeUpsertsTheContactWithTheDataRow() {
        changed(3)
        val id = person().getString("id")

        // The person was renamed and given a new number on the home server, and a server script
        // asks the phone to carry that to the contact.
        home.command(
            "01CMDUPSERT000000000000000",
            "upsert_contact",
            JSONObject()
                .put("contact_id", "3")
                .put("name", "Augusta Ada King")
                .put("phones", JSONArray().put("+61412345678").put("+61411111111"))
                .put("emails", JSONArray().put("ada@example.com"))
                .put("person", id),
        )
        VaultCore.runCommands()

        val done = home.command("01CMDUPSERT000000000000000")
        assertEquals("done", done.getString("status"))
        assertEquals("3", done.getJSONObject("result").getString("contact_id"))
        assertEquals("Augusta Ada King", phone.name(3))
        assertEquals(listOf("+61412345678 2", "+61411111111 2"), phone.rows(3, FakePhone.PHONE))
        assertEquals("the emails agreed already", listOf("ada@example.com"), phone.rows(3, FakePhone.EMAIL))
        val row = vaultRow(3)
        assertTrue(row, row.startsWith("$id "))
        assertEquals("Augusta Ada King", JSONObject(row.substringAfter(" ")).getString("name"))

        // A person with no contact yet gets a new one, holding its id, and a second command for
        // the same person changes that contact rather than adding another.
        val fresh = JSONObject()
            .put("contact_id", JSONObject.NULL)
            .put("name", "Grace Hopper")
            .put("phones", JSONArray().put("+14155552671"))
            .put("emails", JSONArray())
            .put("person", "01PERSONGRACE0000000000000")
        val added = JSONObject(AndroidDeviceData(context, capture).call("contacts_upsert", JSONObject().put("contact", fresh).toString()))
        val grace = added.getJSONObject("ok").getString("contact_id").toLong()
        assertEquals("Grace Hopper", phone.name(grace))
        assertTrue(vaultRow(grace).startsWith("01PERSONGRACE0000000000000 "))
        AndroidDeviceData(context, capture).call("contacts_upsert", JSONObject().put("contact", fresh.put("name", "Grace B. Hopper")).toString())
        assertEquals(listOf(3L, grace), phone.contactIds())
        assertEquals("Grace B. Hopper", phone.name(grace))
    }

    @Test
    fun aPersonMadeOnTheServerBecomesAContactLinkedBothWays() {
        // A person made in the web app reaches the phone with the next round, and the server
        // queues an upsert_contact for it, since no contact here mirrors it yet.
        val carla = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC"
        home.saved(
            carla,
            "person",
            """
            title   = "Carla Created"
            created = "2026-10-12T09:15:00Z"
            updated = "2026-10-12T09:15:00Z"
            phones  = ["+16502530005"]
            """.trimIndent() + "\n",
        )
        VaultCore.sync()
        assertTrue("nothing waits to sync yet", people().isEmpty())
        home.command(
            "01CMDCARLA0000000000000000",
            "upsert_contact",
            JSONObject()
                .put("contact_id", JSONObject.NULL)
                .put("name", "Carla Created")
                .put("phones", JSONArray().put("+16502530005"))
                .put("emails", JSONArray())
                .put("person", carla),
        )
        VaultCore.runCommands()

        // The phone has a new contact, holding the person's id in the app's own row.
        val done = home.command("01CMDCARLA0000000000000000")
        assertEquals("done", done.getString("status"))
        val contact = done.getJSONObject("result").getString("contact_id").toLong()
        assertEquals(listOf(3L, contact), phone.contactIds())
        assertEquals("Carla Created", phone.name(contact))
        assertEquals(listOf("+16502530005 2"), phone.rows(contact, FakePhone.PHONE))
        val row = vaultRow(contact)
        assertTrue(row, row.startsWith("$carla "))
        assertEquals("Carla Created", JSONObject(row.substringAfter(" ")).getString("name"))

        // The person holds the contact's id for this device, waiting to sync to the server.
        val person = person()
        assertEquals(carla, person.getString("id"))
        assertEquals(contact.toString(), links(person)!!.getString("device_${FakeHome.DEVICE_ID}"))
    }

    /** The local vault's assets, through the core, as the app stores them. */
    private object CoreAssets : AssetStore {
        override fun put(filename: String, bytes: ByteArray): JSONObject = JSONObject(VaultCore.putAsset(filename, bytes))

        override fun bytes(asset: String): ByteArray? = VaultCore.assetBytes(asset)
    }

    @Test
    fun anUpsertContactCommandWritesEveryDetailAndItReadsBackUnchanged() {
        phone.photo(3, byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 1))
        changed(3)
        val id = person().getString("id")
        // The contact's photo went into the local vault before its event was queued.
        val event = JSONArray(VaultCore.events()).let { e -> (0 until e.length()).map { e.getJSONObject(it) } }
            .last { it.optString("event") == "contact_changed" }
        val queuedPhoto = event.getJSONObject("data").getJSONObject("photo")
        assertArrayEquals(byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 1), VaultCore.assetBytes(queuedPhoto.getString("asset")))

        val face = byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 42)
        val asset = JSONObject(VaultCore.putAsset("face.jpg", face)).getString("asset")
        val details = JSONObject()
            .put("name", "Augusta Ada King")
            .put("phones", JSONArray().put("+61412345678").put("+442079460018"))
            .put("phone_labels", JSONArray().put("mobile").put("Ockham"))
            .put("emails", JSONArray().put("ada@example.com").put("countess@example.org"))
            .put("email_labels", JSONArray().put("home").put("work"))
            .put("addresses", JSONArray().put("12 St James's Square, London"))
            .put("address_labels", JSONArray().put("home"))
            .put("organisation", "Analytical Society")
            .put("job_title", "Countess of Lovelace")
            .put("nickname", "Enchantress of Numbers")
            .put("birthday", "1815-12-10")
            .put("websites", JSONArray().put("https://ada.example.com"))
            .put("contact_note", "Wrote the first program.")
            .put("photo", asset)
        home.command(
            "01CMDEVERYDETAIL0000000000",
            "upsert_contact",
            JSONObject(details.toString()).put("contact_id", "3").put("person", id),
        )
        VaultCore.runCommands()

        val done = home.command("01CMDEVERYDETAIL0000000000")
        assertEquals(done.toString(), "done", done.getString("status"))
        val read = capture.contacts.get("3")
        for (key in details.keys()) {
            if (key == "photo") continue
            assertEquals(key, details.get(key).toString(), read.get(key).toString())
        }
        assertEquals("the photo reads back as the asset it was written from", asset, read.getJSONObject("photo").getString("asset"))
        assertArrayEquals(face, phone.photoOf(3))
        val synced = read.getJSONObject("synced")
        assertEquals("1815-12-10", synced.getString("birthday"))
        assertEquals("[\"home\",\"work\"]", synced.getJSONArray("email_labels").toString())
        assertEquals(asset, synced.getJSONObject("photo").getString("asset"))
    }

    @Test
    fun deletingTheContactUnlinksThePersonAndKeepsIt() {
        changed(3)
        val id = person().getString("id")

        phone.deleteContact(3, 9_000)
        VaultCore.enqueue("contact_deleted", JSONObject().put("contact_id", "3").toString(), System.currentTimeMillis())
        VaultCore.runPending()

        val ada = person()
        assertEquals(id, ada.getString("id"))
        assertEquals("Ada Lovelace", ada.getJSONObject("fields").getString("title"))
        val links = links(ada)
        assertTrue("$links", links == null || !links.has("device_${FakeHome.DEVICE_ID}"))
    }
}
