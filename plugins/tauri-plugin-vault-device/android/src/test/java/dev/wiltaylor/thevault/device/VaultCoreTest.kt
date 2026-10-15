package dev.wiltaylor.thevault.device

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.BeforeClass
import org.junit.Test
import java.nio.file.Files

/**
 * S5: Kotlin calls into the Rust core over JNI, with no webview, and captured events land in the
 * device event queue. The core is one per process, so every test here shares one vault.
 *
 * Plain JUnit rather than Robolectric: the JVM loads the native library once, in one class loader.
 */
class VaultCoreTest {
    companion object {
        /** What the device module asked of the phone, as `method args`. */
        val asked: MutableList<String> = java.util.Collections.synchronizedList(mutableListOf())

        @BeforeClass
        @JvmStatic
        fun openCore() {
            assertNotNull(
                "vault.core.library must name a host build of the plugin's Rust library; run `just android-test`",
                System.getProperty("vault.core.library"),
            )
            val vault = Files.createTempDirectory("vault-core-test").resolve("vault")
            // A share handler that asks the phone for its contacts, through the bridge below.
            Files.createDirectories(vault.resolve("scripts"))
            Files.write(
                vault.resolve("scripts/device_share.ws"),
                (
                    "use device\n\n" +
                        "fn share_received(event: Value) -> Result[(), string] {\n" +
                        "    device::contacts_list(event.get(\"data\").unwrap())?;\n" +
                        "    Ok(())\n" +
                        "}\n"
                    ).toByteArray(),
            )
            VaultCore.open(vault.toString())
            openDevice()
        }

        private fun openDevice() {
            VaultCore.openDevice(DEVICE, DeviceBridge { method, args ->
                asked.add("$method $args")
                if (method == "contacts_list") """{"ok":[]}""" else """{"err":"not in this test"}"""
            })
        }

        private const val DEVICE = "01JDEVICE0000000000000000"
    }

    /**
     * Queued event [id] as JSON text, from `VaultCore.events()`. Tests share the queue, so each
     * finds its own event rather than reading what a pass ran.
     */
    private fun queued(id: Long): String {
        val events = VaultCore.events()
        // Each event is one object whose payload holds no nested object in these tests.
        return Regex("\\{[^{}]*(\\{[^{}]*\\}[^{}]*)*\\}").findAll(events.removePrefix("[").removeSuffix("]"))
            .map { it.value }
            .firstOrNull { Regex("\"id\":$id[,}]").containsMatchIn(it) }
            ?: throw AssertionError("no event $id in $events")
    }

    @Test
    fun openingTheDeviceAgainKeepsTheQueue() {
        val id = VaultCore.enqueue("call_ended", """{"device_ref":"call:78"}""", 1)
        openDevice()
        assertTrue(queued(id).contains("\"device_ref\":\"call:78\""))
    }

    @Test
    fun aHandlerReachesThePhoneThroughTheBridge() {
        val id = VaultCore.enqueue("share_received", """{"text":"x","subject":"","url":null,"attachments":[]}""", 1)
        VaultCore.runPending()
        assertTrue(queued(id).contains("\"status\":\"done\""))
        assertTrue("asked: $asked", asked.any { it.startsWith("contacts_list ") && it.contains("\"text\":\"x\"") })
    }

    @Test
    fun pingRoundTripsThroughRust() {
        val answer = VaultCore.ping("hello from Kotlin")
        assertTrue("unexpected answer: $answer", answer.startsWith("tauri-plugin-vault-device "))
        assertTrue("unexpected answer: $answer", answer.endsWith(": hello from Kotlin"))
    }

    @Test
    fun pingCarriesUnicode() {
        assertEquals("ünïcødé ✓", VaultCore.ping("ünïcødé ✓").substringAfter(": "))
    }

    @Test
    fun anEnqueuedEventIsStoredInTheQueueAndRuns() {
        val data = """{"device_ref":"call:77","direction":"missed","number":"+61412345678","number_raw":"0412345678","contact":null,"started":"2026-10-20T09:30:00+11:00","duration":0}"""
        val id = VaultCore.enqueue("call_ended", data, 1_792_449_000_000)
        assertTrue(id > 0)

        val stored = queued(id)
        assertTrue(stored, stored.contains("\"device_ref\":\"call:77\""))
        assertTrue(stored, stored.contains("\"event\":\"call_ended\""))

        VaultCore.runPending()
        assertTrue(queued(id).contains("\"status\":\"done\""))
    }

    @Test
    fun anUnknownEventIsRefusedWithTheCoresMessage() {
        try {
            VaultCore.enqueue("sms_exploded", "{}", 0)
            fail("enqueue accepted an unknown event")
        } catch (e: IllegalStateException) {
            assertTrue(e.message!!, e.message!!.contains("is not a device event"))
        }
        try {
            VaultCore.enqueue("sms_received", "not json", 0)
            fail("enqueue accepted bad JSON")
        } catch (e: IllegalStateException) {
            assertNotNull(e.message)
        }
    }

    @Test
    fun anAttachmentIsStoredAsAnAssetInTheLocalVault() {
        val stored = VaultCore.putAsset("photo.jpg", byteArrayOf(1, 2, 3))
        assertTrue(stored, Regex("\"asset\":\"[0-9A-Z]{26}\"").containsMatchIn(stored))
        assertTrue(stored, stored.contains("\"filename\":\"photo.jpg\""))
        assertTrue(stored, stored.contains("\"size\":3"))
    }
}
