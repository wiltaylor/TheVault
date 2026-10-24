package dev.wiltaylor.thevault.device

import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Parcel
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

    /** [intent] as the share target receives it from another app: parcelled through Binder. */
    private fun delivered(intent: Intent): Intent {
        val parcel = Parcel.obtain()
        try {
            intent.writeToParcel(parcel, 0)
            parcel.setDataPosition(0)
            return Intent.CREATOR.createFromParcel(parcel)
        } finally {
            parcel.recycle()
        }
    }

    /** The kinds of the blocks in the note's body. */
    private fun bodyKinds(note: JSONObject): List<String> {
        val blocks = note.getJSONArray("blocks")
        if (blocks.length() == 0) return emptyList()
        val body = blocks.getJSONObject(0).getJSONArray("blocks")
        return (0 until body.length()).map { body.getJSONObject(it).getString("kind") }
    }

    @Test
    fun oneSharedPdfIsAttachedInTheBody() {
        val intent = Intent(Intent.ACTION_SEND).setType("application/pdf")
            .putExtra(Intent.EXTRA_STREAM, shared("invoice.pdf", "%PDF-1.4 invoice".toByteArray()))
        val note = share(delivered(intent))
        assertEquals(listOf("invoice.pdf"), filenames(note))
        assertFalse("a PDF is not an image", note.getJSONObject("fields").has("image"))
        assertEquals(listOf("asset"), bodyKinds(note))
    }

    @Test
    fun filesAnAppSharesFromAnotherProcessAreAttached() {
        // As a real share arrives: the intent crossed Binder, so its extras are read back lazily.
        val one = Intent(Intent.ACTION_SEND).setType("image/jpeg")
            .putExtra(Intent.EXTRA_STREAM, shared("IMG_0003.jpg", ByteArray(32)))
        val note = share(delivered(one))
        assertEquals(listOf("IMG_0003.jpg"), filenames(note))
        assertEquals(1, note.getJSONObject("fields").getJSONArray("image").length())

        val many = Intent(Intent.ACTION_SEND_MULTIPLE).setType("*/*")
            .putParcelableArrayListExtra(
                Intent.EXTRA_STREAM,
                arrayListOf(shared("slides.pdf", "%PDF-1.4".toByteArray()), shared("IMG_0004.png", ByteArray(16))),
            )
        val both = share(delivered(many))
        assertEquals(listOf("IMG_0004.png", "slides.pdf"), filenames(both).sorted())
        assertEquals(1, both.getJSONObject("fields").getJSONArray("image").length())
        assertEquals(listOf("asset"), bodyKinds(both))
    }

    @Test
    fun filesSharedOnlyAsClipDataAreAttached() {
        // Some apps hand their files over in the intent's ClipData and leave EXTRA_STREAM out,
        // often with a line of text: the note must not keep the text and drop the files.
        val clip = ClipData.newRawUri("shared", shared("minutes.pdf", "%PDF-1.4 minutes".toByteArray()))
        clip.addItem(ClipData.Item(shared("whiteboard.jpg", ByteArray(48))))
        val intent = Intent(Intent.ACTION_SEND_MULTIPLE).setType("*/*")
            .putExtra(Intent.EXTRA_TEXT, "From the meeting")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        intent.clipData = clip
        val note = share(delivered(intent))
        assertEquals(listOf("minutes.pdf", "whiteboard.jpg"), filenames(note).sorted())
        assertEquals(1, note.getJSONObject("fields").getJSONArray("image").length())
        assertEquals(listOf("paragraph", "asset"), bodyKinds(note))
    }

    @Test
    fun aFileInBothExtraStreamAndClipDataIsAttachedOnce() {
        // Android copies EXTRA_STREAM into the ClipData when the sharing app starts the share.
        val uri = shared("report.pdf", "%PDF-1.4".toByteArray())
        val intent = Intent(Intent.ACTION_SEND).setType("application/pdf").putExtra(Intent.EXTRA_STREAM, uri)
        intent.clipData = ClipData.newRawUri("", uri)
        val note = share(delivered(intent))
        assertEquals(listOf("report.pdf"), filenames(note))
    }

    @Test
    fun aSingleFileSentAsAListIsAttached() {
        val intent = Intent(Intent.ACTION_SEND).setType("application/pdf")
            .putParcelableArrayListExtra(Intent.EXTRA_STREAM, arrayListOf(shared("form.pdf", "%PDF-1.4".toByteArray())))
        val note = share(delivered(intent))
        assertEquals(listOf("form.pdf"), filenames(note))
    }

    @Test
    fun theSharedTypeNamesAFileItsProviderDoesNotType() {
        // A provider that types nothing and names its file with no extension: the share's own
        // type says it is a photo, so it shows as the note's image.
        val intent = Intent(Intent.ACTION_SEND).setType("image/jpeg")
            .putExtra(Intent.EXTRA_STREAM, shared("1000012345", ByteArray(24)))
        val outcome = ShareImporter.import(context, delivered(intent))!!
        assertEquals("image/jpeg", attachments(outcome).getJSONObject(0).getString("mime"))
        val note = queued(outcome.getJSONArray("notes").getString(0))
        assertEquals(listOf("1000012345.jpg"), filenames(note))
        assertEquals(1, note.getJSONObject("fields").getJSONArray("image").length())
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
