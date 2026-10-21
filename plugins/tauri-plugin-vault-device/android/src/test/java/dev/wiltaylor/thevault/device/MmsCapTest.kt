package dev.wiltaylor.thevault.device

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.work.Configuration
import androidx.work.testing.SynchronousExecutor
import androidx.work.testing.WorkManagerTestInitHelper
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.io.ByteArrayInputStream
import java.io.File
import java.io.InputStream

/**
 * S5: an MMS part is streamed into the local vault rather than read whole into memory, and one
 * over the cap is not stored but listed in the message's body by name and size (ticket 141).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class MmsCapTest {
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
        PairingStore(context).save(
            JSONObject()
                .put("server_url", home.url)
                .put("device_id", FakeHome.DEVICE_ID)
                .put("device_name", "Pixel")
                .put("token", FakeHome.TOKEN),
        )
        phone = FakePhone(context)
    }

    @After
    fun tearDown() {
        Device.reset()
        home.close()
    }

    private fun bytes(n: Int) = ByteArray(n) { (it % 251).toByte() }

    private fun reader(assets: AssetStore, cap: Long) =
        MmsReader(context.contentResolver, PhoneNumbers("AU"), ContactLookup(context.contentResolver), assets, { emptySet() }, cap)

    @Test
    fun aPartOverTheCapIsListedInTheBodyAndNotStored() {
        phone.mms(7, "+61412345678", listOf(FakePhone.SELF))
        phone.part(70, 7, "text/plain", text = "holiday clips")
        phone.part(71, 7, "image/jpeg", name = "small.jpg", bytes = bytes(900))
        phone.part(72, 7, "video/mp4", name = "clip.mp4", bytes = bytes(5_000))

        val assets = RecordingAssets()
        val mms = reader(assets, cap = 1_000).since(0).single()

        assertEquals(listOf("small.jpg"), assets.stored.map { it.first })
        val attachments = mms.data.getJSONArray("attachments")
        assertEquals(1, attachments.length())
        assertEquals("small.jpg", attachments.getJSONObject(0).getString("filename"))
        assertEquals(900L, attachments.getJSONObject(0).getLong("size"))
        assertEquals(
            "holiday clips\nAttachments not stored, too large: clip.mp4 (5 KB)",
            mms.data.getString("body"),
        )
    }

    @Test
    fun theCapIs50MbAndSizesReadAsTheGmailImportWritesThem() {
        assertEquals(50L * 1024 * 1024, MmsReader.MAX_ATTACHMENT)
        assertEquals("72.0 MB", sizeText(72L * 1024 * 1024))
        assertEquals("800 KB", sizeText(800L * 1024))
    }

    @Test
    fun aPartStreamsThroughAFileIntoTheVault() {
        assertTrue(Device.openCore(context))
        val store = Device.coreAssets(context)
        val dir = File(context.filesDir, "incoming")

        // A part read a block at a time: the store never asks for it whole.
        val part = bytes(3 * 64 * 1024 + 17)
        var biggestRead = 0
        val input = object : InputStream() {
            val inner = ByteArrayInputStream(part)
            override fun read(): Int = inner.read()
            override fun read(b: ByteArray, off: Int, len: Int): Int {
                val n = inner.read(b, off, len)
                if (n > biggestRead) biggestRead = n
                return n
            }
        }
        val stored = store.putStream("clip.mp4", input, MmsReader.MAX_ATTACHMENT)
        assertTrue(stored.toString(), stored.has("asset"))
        assertEquals(part.size.toLong(), stored.getLong("size"))
        assertTrue("read $biggestRead bytes at once", biggestRead <= 64 * 1024)
        assertArrayEquals(part, VaultCore.assetBytes(stored.getString("asset")))
        // The file it went through was moved into the vault.
        assertEquals(0, dir.listFiles()?.size ?: 0)

        // Over the cap, nothing is stored and the file is gone.
        val over = store.putStream("huge.mp4", ByteArrayInputStream(bytes(2_000)), 1_000)
        assertFalse(over.has("asset"))
        assertEquals(2_000L, over.getLong("size"))
        assertEquals(0, dir.listFiles()?.size ?: 0)
    }
}
