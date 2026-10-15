package dev.wiltaylor.thevault.device

import android.Manifest
import android.app.Activity
import android.app.Application
import android.content.pm.PackageManager
import android.content.Context
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import androidx.work.testing.WorkManagerTestInitHelper
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * Onboarding pairs through the Rust core and keeps the token sealed (D2), then asks to post
 * notifications (D9).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class OnboardingActivityTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val home = FakeHome()

    @Before
    fun setUp() {
        PairingStore.defaultCipher = FakeCipher
        WorkManagerTestInitHelper.initializeTestWorkManager(context)
        shadowOf(context as Application).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
        // Open this test's local vault before the activity pairs, as the app has by the time
        // onboarding shows. A fresh vault writes state.db and the built-in types with an fsync
        // each, which under disk pressure from other builds takes seconds; done inside the pairing
        // job, it ate the ten seconds [waitFor] allows. Pairing then finds the vault open.
        Vault.open(context)
    }

    @After
    fun stop() {
        // Pairing kicked the foreground loop: drop the link so its round, due after the settle
        // delay, does not run against the closed fake home server while the next test opens its
        // vault.
        VaultCore.link(null)
        home.close()
    }

    /** Runs the main looper until [done] or ten seconds pass. */
    private fun waitFor(done: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 10_000
        while (!done() && System.currentTimeMillis() < deadline) {
            shadowOf(Looper.getMainLooper()).idle()
            Thread.sleep(20)
        }
    }

    @Test
    fun pairingByPasswordStoresTheLinkAndCloses() {
        val controller = Robolectric.buildActivity(OnboardingActivity::class.java).setup()
        val activity = controller.get()
        activity.serverUrl.setText(home.url + "/")
        activity.password.setText(FakeHome.PASSWORD)
        activity.deviceName.setText("Pixel 9")
        activity.cacheMb.setText("128")
        activity.pairButton.performClick()
        waitFor { activity.isFinishing }
        assertTrue(activity.message.text.toString(), activity.isFinishing)
        assertEquals(Activity.RESULT_OK, shadowOf(activity).resultCode)
        val pair = home.bodies[home.requests.indexOf("POST /api/devices/pair")]
        assertTrue(pair, pair.contains("\"name\":\"Pixel 9\"") && pair.contains("\"platform\":\"android\""))
        val link = PairingStore(context).link()!!
        assertEquals(FakeHome.TOKEN, link.getString("token"))
        assertEquals(home.url, link.getString("server_url"))
        assertEquals(128L, link.getLong("cache_mb"))
        assertTrue(org.json.JSONObject(VaultCore.status()).getBoolean("paired"))
    }

    @Test
    fun aScannedCodePairsWithoutThePassword() {
        val activity = Robolectric.buildActivity(OnboardingActivity::class.java).setup().get()
        activity.pair(home.url, null, FakeHome.CODE)
        waitFor { activity.isFinishing }
        assertTrue(activity.message.text.toString(), activity.isFinishing)
        assertTrue(PairingStore(context).isPaired)
    }

    @Test
    fun aWrongPasswordSaysSoAndStoresNothing() {
        val activity = Robolectric.buildActivity(OnboardingActivity::class.java).setup().get()
        activity.serverUrl.setText(home.url)
        activity.password.setText("wrong")
        activity.pairButton.performClick()
        waitFor { activity.message.text.contains("wrong password") }
        assertEquals("wrong password", activity.message.text.toString())
        assertFalse(activity.isFinishing)
        assertFalse(PairingStore(context).isPaired)
    }

    @Test
    fun afterPairingItAsksToPostNotificationsAndClosesWhateverTheAnswer() {
        shadowOf(context as Application).denyPermissions(Manifest.permission.POST_NOTIFICATIONS)
        val activity = Robolectric.buildActivity(OnboardingActivity::class.java).setup().get()
        activity.pair(home.url, FakeHome.PASSWORD, null)
        waitFor { shadowOf(activity).lastRequestedPermission != null }
        val asked = shadowOf(activity).lastRequestedPermission!!
        assertEquals(listOf(Manifest.permission.POST_NOTIFICATIONS), asked.requestedPermissions.toList())
        assertFalse(activity.isFinishing)
        assertTrue(PairingStore(context).isPaired)

        activity.onRequestPermissionsResult(
            asked.requestCode,
            arrayOf(Manifest.permission.POST_NOTIFICATIONS),
            intArrayOf(PackageManager.PERMISSION_DENIED),
        )
        waitFor { activity.isFinishing }
        assertTrue(activity.isFinishing)
        assertEquals(Activity.RESULT_OK, shadowOf(activity).resultCode)
    }
}
