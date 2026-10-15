package dev.wiltaylor.thevault.device

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.test.core.app.ApplicationProvider
import java.io.ByteArrayInputStream
import java.io.File
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * S5: share intents with text, a link, one image and several files run as `share_received`
 * device events, whose default handler (`scripts/device_share.ws`) makes one fleeting note each
 * in the local vault, queued for the home server, with the shared files as their assets. The
 * device is not paired, so this is the offline case: nothing leaves the phone.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class ShareTargetTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    @Before
    fun openTheVault() {
        PairingStore.defaultCipher = FakeCipher
        Device.reset()
        Vault.open(context)
    }

    /** A `content://` file another app shares, with these bytes. */
    private fun shared(name: String, bytes: ByteArray): Uri {
        val uri = Uri.parse("content://com.example.gallery/files/$name")
        shadowOf(context.contentResolver).registerInputStream(uri, ByteArrayInputStream(bytes))
        return uri
    }

    /** The queued note [id], from the core. */
    private fun queued(id: String): JSONObject {
        val all = JSONArray(VaultCore.queued())
        for (i in 0 until all.length()) {
            if (all.getJSONObject(i).getString("id") == id) return all.getJSONObject(i)
        }
        throw AssertionError("note $id is not queued: $all")
    }

    /** Queued event [id], from `VaultCore.events()`. */
    private fun event(id: Long): JSONObject {
        val all = JSONArray(VaultCore.events())
        for (i in 0 until all.length()) {
            if (all.getJSONObject(i).getLong("id") == id) return all.getJSONObject(i)
        }
        throw AssertionError("no event $id: $all")
    }

    /**
     * Shares [intent] and answers the one note it made, after checking it came from the
     * `share_received` handler run through the device event queue, not the core's fallback.
     */
    private fun share(intent: Intent): JSONObject {
        val outcome = ShareImporter.import(context, intent)!!
        assertEquals(outcome.toString(), "done", outcome.getString("status"))
        assertFalse("the share fell back instead of running its handler: $outcome", outcome.isNull("event"))
        val event = event(outcome.getLong("event"))
        assertEquals("share_received", event.getString("event"))
        assertEquals("done", event.getString("status"))
        val notes = outcome.getJSONArray("notes")
        assertEquals("exactly one note per share: $outcome", 1, notes.length())
        return queued(notes.getString(0))
    }

    /** The attachments of the `share_received` event a share [outcome] names. */
    private fun attachments(outcome: JSONObject): JSONArray =
        event(outcome.getLong("event")).getJSONObject("data").getJSONArray("attachments")

    private fun filenames(note: JSONObject): List<String> {
        val assets = note.getJSONArray("assets")
        return (0 until assets.length()).map { assets.getJSONObject(it).getString("filename") }
    }

    /** The inline markup of every paragraph in the note's body, one per line. */
    private fun bodyText(note: JSONObject): String {
        val out = mutableListOf<String>()
        fun walk(blocks: JSONArray) {
            for (i in 0 until blocks.length()) {
                val b = blocks.getJSONObject(i)
                b.optJSONObject("fields")?.optString("text")?.takeIf { it.isNotEmpty() }?.let { out.add(it) }
                b.optJSONArray("blocks")?.let { walk(it) }
            }
        }
        walk(note.getJSONArray("blocks"))
        return out.joinToString("\n")
    }

    @Test
    fun sharedTextBecomesAFleetingNote() {
        val intent = Intent(Intent.ACTION_SEND).setType("text/plain")
            .putExtra(Intent.EXTRA_TEXT, "Buy *oat* milk\n\nand bread")
        val note = share(intent)
        assertEquals("fleeting", note.getString("type"))
        assertEquals("Buy *oat* milk", note.getJSONObject("fields").getString("title"))
        assertFalse(note.getJSONObject("fields").getBoolean("processed"))
        assertEquals("Buy \\*oat\\* milk\nand bread", bodyText(note))
        assertEquals(emptyList<String>(), filenames(note))
    }

    @Test
    fun aSharedLinkKeepsItsPageTitle() {
        val intent = Intent(Intent.ACTION_SEND).setType("text/plain")
            .putExtra(Intent.EXTRA_SUBJECT, "Rust 2024 is out")
            .putExtra(Intent.EXTRA_TEXT, "https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html")
        val note = share(intent)
        assertEquals("Rust 2024 is out", note.getJSONObject("fields").getString("title"))
        assertEquals("https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html", bodyText(note))
    }

    @Test
    fun oneSharedImageBecomesTheNotesAsset() {
        val bytes = ByteArray(4096) { (it % 251).toByte() }
        val intent = Intent(Intent.ACTION_SEND).setType("image/jpeg")
            .putExtra(Intent.EXTRA_STREAM, shared("IMG_0001.jpg", bytes))
        val note = share(intent)
        assertEquals(listOf("IMG_0001.jpg"), filenames(note))
        assertEquals(4096, note.getJSONArray("assets").getJSONObject(0).getInt("size"))
        assertEquals("IMG_0001.jpg", note.getJSONObject("fields").getString("title"))
        val image = note.getJSONObject("fields").getJSONArray("image")
        assertEquals(note.getJSONArray("assets").getJSONObject(0).getString("id"), image.getString(0))
        // The copy in the cache folder is gone once the core stored it.
        assertEquals(0, File(context.cacheDir, "share").listFiles()!!.size)
    }

    @Test
    fun severalSharedFilesBecomeOneNote() {
        val uris = arrayListOf(
            shared("report.pdf", "%PDF-1.4 report".toByteArray()),
            shared("data.csv", "a,b\n1,2\n".toByteArray()),
            shared("photo.png", ByteArray(100)),
        )
        val intent = Intent(Intent.ACTION_SEND_MULTIPLE).setType("*/*")
            .putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris)
        val note = share(intent)
        assertEquals(listOf("data.csv", "photo.png", "report.pdf"), filenames(note).sorted())
        // The default handler shows the image and attaches the other files after the text.
        val image = note.getJSONObject("fields").getJSONArray("image")
        assertEquals(1, image.length())
        val photo = (0 until 3).map { note.getJSONArray("assets").getJSONObject(it) }
            .single { it.getString("filename") == "photo.png" }
        assertEquals(photo.getString("id"), image.getString(0))
        val body = note.getJSONArray("blocks").getJSONObject(0).getJSONArray("blocks")
        assertEquals(listOf("asset", "asset"), (0 until body.length()).map { body.getJSONObject(it).getString("kind") })
        assertEquals("report.pdf", note.getJSONObject("fields").getString("title"))
    }

    @Test
    fun sharedFilesReachTheHandlerAsAttachments() {
        val uris = arrayListOf(
            shared("scan.pdf", "%PDF-1.4 scan".toByteArray()),
            shared("IMG_0002.png", ByteArray(64)),
        )
        val intent = Intent(Intent.ACTION_SEND_MULTIPLE).setType("*/*")
            .putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris)
        val outcome = ShareImporter.import(context, intent)!!
        val files = attachments(outcome)
        assertEquals(2, files.length())
        val pdf = files.getJSONObject(0)
        assertEquals("scan.pdf", pdf.getString("filename"))
        assertEquals("application/pdf", pdf.getString("mime"))
        assertEquals(13, pdf.getInt("size"))
        assertEquals("image/png", files.getJSONObject(1).getString("mime"))
        // Each attachment is an asset of the local vault, which the note names.
        val note = queued(outcome.getJSONArray("notes").getString(0))
        val ids = (0 until 2).map { note.getJSONArray("assets").getJSONObject(it).getString("id") }.toSet()
        assertEquals(setOf(pdf.getString("asset"), files.getJSONObject(1).getString("asset")), ids)
    }

    @Test
    fun anEmptyShareSavesNothing() {
        assertNull(ShareImporter.import(context, Intent(Intent.ACTION_SEND).setType("text/plain")))
        assertNull(ShareImporter.import(context, Intent(Intent.ACTION_VIEW)))
    }

    @Test
    fun theShareActivityTakesEveryKindOfShare() {
        val pm = context.packageManager
        for (type in listOf("text/plain", "image/jpeg", "application/pdf")) {
            val send = Intent(Intent.ACTION_SEND).setType(type).setPackage(context.packageName)
            assertEquals(type, ShareActivity::class.java.name, pm.queryIntentActivities(send, 0).single().activityInfo.name)
        }
        val many = Intent(Intent.ACTION_SEND_MULTIPLE).setType("image/*").setPackage(context.packageName)
        assertEquals(ShareActivity::class.java.name, pm.queryIntentActivities(many, 0).single().activityInfo.name)
    }
}
