package dev.wiltaylor.thevault.device

import android.app.Activity
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.shadows.ShadowAlertDialog
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class HttpWarningTest {
    @Test
    fun existingHttpPairingShowsWarningWithoutChangingPairing() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        assertTrue(HttpWarning.show(activity, "http://public.example.com"))
        val dialog = ShadowAlertDialog.getLatestAlertDialog()
        assertTrue(dialog.isShowing)
        assertEquals(HttpWarning.MESSAGE, org.robolectric.Shadows.shadowOf(dialog).message.toString())
        dialog.dismiss()
        assertFalse(HttpWarning.show(activity, "https://public.example.com"))
        assertFalse(HttpWarning.show(activity, null))
    }
}
