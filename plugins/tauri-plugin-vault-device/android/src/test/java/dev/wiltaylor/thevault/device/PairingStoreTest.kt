package dev.wiltaylor.thevault.device

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** The pairing at rest: the token only ever sealed (D2), and the QR payload. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class PairingStoreTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    private val paired = JSONObject()
        .put("server_url", "https://vault.example.net")
        .put("device_id", "01J9Z3K8Q2W6E4R5T7Y8U9I0OP")
        .put("device_name", "Pixel")
        .put("token", "tv_secret_token")

    @Test
    fun theTokenIsStoredSealedAndComesBackInTheLink() {
        val store = PairingStore(context, FakeCipher)
        assertFalse(store.isPaired)
        assertNull(store.link())
        store.save(paired, 64)
        assertTrue(store.isPaired)
        val prefs = context.getSharedPreferences(PairingStore.PREFS, Context.MODE_PRIVATE).all
        assertFalse(prefs.values.any { it.toString().contains("tv_secret_token") })
        val link = store.link()!!
        assertEquals("tv_secret_token", link.getString("token"))
        assertEquals("https://vault.example.net", link.getString("server_url"))
        assertEquals("Pixel", link.getString("device_name"))
        assertEquals(64L, link.getLong("cache_mb"))
        store.clear()
        assertNull(store.link())
    }

    @Test
    fun aTokenThatCannotBeUnsealedMeansPairingAgain() {
        PairingStore(context, FakeCipher).save(paired)
        val broken = object : TokenCipher {
            override fun encrypt(plain: ByteArray) = plain
            override fun decrypt(sealed: ByteArray): ByteArray = throw javax.crypto.AEADBadTagException()
        }
        assertNull(PairingStore(context, broken).link())
    }

    @Test
    fun readsThePairingQrPayload() {
        assertEquals(
            PairingQr("https://v.example.net", "ABCD-1234"),
            PairingQr.parse("""{"server_url":"https://v.example.net","code":"ABCD-1234"}"""),
        )
        assertNull(PairingQr.parse("""{"server_url":"ftp://x","code":"A"}"""))
        assertNull(PairingQr.parse("""{"server_url":"https://x"}"""))
        assertNull(PairingQr.parse("https://not-json.example"))
    }
}
