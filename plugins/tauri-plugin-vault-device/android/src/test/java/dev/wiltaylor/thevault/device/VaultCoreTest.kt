package dev.wiltaylor.thevault.device

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** S5: Kotlin calls into the Rust core over JNI, with no webview. */
class VaultCoreTest {
    @Test
    fun pingRoundTripsThroughRust() {
        assertNotNull(
            "vault.core.library must name a host build of the plugin's Rust library; run `just android-test`",
            System.getProperty("vault.core.library"),
        )
        val answer = VaultCore.ping("hello from Kotlin")
        assertTrue("unexpected answer: $answer", answer.startsWith("tauri-plugin-vault-device "))
        assertTrue("unexpected answer: $answer", answer.endsWith(": hello from Kotlin"))
    }

    @Test
    fun pingCarriesUnicode() {
        assertEquals("ünïcødé ✓", VaultCore.ping("ünïcødé ✓").substringAfter(": "))
    }
}
