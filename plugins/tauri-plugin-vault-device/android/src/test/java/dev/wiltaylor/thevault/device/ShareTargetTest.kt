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
 * S5: share intents with text, a link, one image and several files become fleeting notes in the
 * local vault, queued for the home server, with the shared files as their assets. The device is
 * not paired, so this is the offline case: nothing leaves the phone.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class ShareTargetTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    @Before
    fun openTheVault() {
        PairingStore.defaultCipher = FakeCipher
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
        val note = queued(ShareImporter.import(context, intent)!!)
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
        val note = queued(ShareImporter.import(context, intent)!!)
        assertEquals("Rust 2024 is out", note.getJSONObject("fields").getString("title"))
        assertEquals("https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html", bodyText(note))
    }

    @Test
    fun oneSharedImageBecomesTheNotesAsset() {
        val bytes = ByteArray(4096) { (it % 251).toByte() }
        val intent = Intent(Intent.ACTION_SEND).setType("image/jpeg")
            .putExtra(Intent.EXTRA_STREAM, shared("IMG_0001.jpg", bytes))
        val note = queued(ShareImporter.import(context, intent)!!)
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
        val note = queued(ShareImporter.import(context, intent)!!)
        assertEquals(listOf("report.pdf", "data.csv", "photo.png"), filenames(note))
        assertEquals(3, note.getJSONObject("fields").getJSONArray("image").length())
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
