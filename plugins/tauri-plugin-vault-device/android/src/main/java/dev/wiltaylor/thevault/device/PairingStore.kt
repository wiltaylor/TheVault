package dev.wiltaylor.thevault.device

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import org.json.JSONObject

/** Encrypts the device token at rest. */
interface TokenCipher {
    fun encrypt(plain: ByteArray): ByteArray

    fun decrypt(sealed: ByteArray): ByteArray
}

/**
 * AES-GCM under a key that never leaves the Android Keystore (D2). The sealed form is the
 * 12-byte IV followed by the ciphertext.
 */
object KeystoreCipher : TokenCipher {
    private const val KEYSTORE = "AndroidKeyStore"
    private const val ALIAS = "thevault-device-token"
    private const val TRANSFORM = "AES/GCM/NoPadding"
    private const val IV_BYTES = 12

    private fun key(): SecretKey {
        val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    override fun encrypt(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORM)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        return cipher.iv + cipher.doFinal(plain)
    }

    override fun decrypt(sealed: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORM)
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, sealed, 0, IV_BYTES))
        return cipher.doFinal(sealed, IV_BYTES, sealed.size - IV_BYTES)
    }

    /** Forgets the key, so a token sealed with it can no longer be read. */
    fun forget() {
        KeyStore.getInstance(KEYSTORE).apply { load(null) }.deleteEntry(ALIAS)
    }
}

/**
 * The device's pairing with its home server. The server URL, device id and name and the asset
 * cache size are plain preferences; the token is only ever stored sealed by [cipher].
 */
class PairingStore(context: Context, private val cipher: TokenCipher = defaultCipher) {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** Stores the answer of [VaultCore.pair]. */
    fun save(paired: JSONObject, cacheMb: Long = DEFAULT_CACHE_MB) {
        val sealed = cipher.encrypt(paired.getString("token").toByteArray(Charsets.UTF_8))
        prefs.edit()
            .putString(SERVER_URL, paired.getString("server_url"))
            .putString(DEVICE_ID, paired.getString("device_id"))
            .putString(DEVICE_NAME, paired.getString("device_name"))
            .putString(TOKEN, Base64.encodeToString(sealed, Base64.NO_WRAP))
            .putLong(CACHE_MB, cacheMb.coerceAtLeast(1))
            .commit()
    }

    val isPaired: Boolean
        get() = prefs.contains(TOKEN)

    /** The device id pairing gave this phone, or null before pairing. */
    val deviceId: String?
        get() = prefs.getString(DEVICE_ID, null)?.takeIf { isPaired && it.isNotEmpty() }

    var cacheMb: Long
        get() = prefs.getLong(CACHE_MB, DEFAULT_CACHE_MB)
        set(value) {
            prefs.edit().putLong(CACHE_MB, value.coerceAtLeast(1)).commit()
        }

    /**
     * The pairing as the core's link JSON, or null when the device is not paired or its token can
     * no longer be unsealed (the Keystore key was lost), which means pairing again.
     */
    fun link(): JSONObject? {
        val sealed = prefs.getString(TOKEN, null) ?: return null
        val token = try {
            String(cipher.decrypt(Base64.decode(sealed, Base64.NO_WRAP)), Charsets.UTF_8)
        } catch (e: Exception) {
            return null
        }
        return JSONObject()
            .put("server_url", prefs.getString(SERVER_URL, ""))
            .put("token", token)
            .put("device_id", prefs.getString(DEVICE_ID, ""))
            .put("device_name", prefs.getString(DEVICE_NAME, ""))
            .put("cache_mb", cacheMb)
    }

    fun clear() {
        prefs.edit().clear().commit()
    }

    companion object {
        const val PREFS = "thevault-pairing"
        const val DEFAULT_CACHE_MB = 512L
        private const val SERVER_URL = "server_url"
        private const val DEVICE_ID = "device_id"
        private const val DEVICE_NAME = "device_name"
        private const val TOKEN = "token_sealed"
        private const val CACHE_MB = "asset_cache_mb"

        /** The cipher stores use; tests replace the Keystore, which the JVM does not have. */
        @JvmStatic
        var defaultCipher: TokenCipher = KeystoreCipher
    }
}

/** What a pairing QR code from Settings > Devices carries: `{"server_url", "code"}`. */
data class PairingQr(val serverUrl: String, val code: String) {
    companion object {
        /** The QR code's payload, or null when it is not a TheVault pairing code. */
        fun parse(text: String): PairingQr? = try {
            val json = JSONObject(text.trim())
            val url = json.optString("server_url")
            val code = json.optString("code")
            if ((url.startsWith("https://") || url.startsWith("http://")) && code.isNotBlank()) {
                PairingQr(url, code)
            } else {
                null
            }
        } catch (e: Exception) {
            null
        }
    }
}
