package dev.wiltaylor.thevault.device

import android.app.Activity
import android.view.ViewGroup
import android.webkit.WebView
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** Ticket 79: the page learns the system bar insets, and the keyboard lifts the webview. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33], qualifiers = "xxhdpi")
class SystemBarsTest {
    private fun insets(top: Int, bottom: Int, keyboard: Int = 0): WindowInsetsCompat =
        WindowInsetsCompat.Builder()
            .setInsets(WindowInsetsCompat.Type.systemBars(), Insets.of(0, top, 0, bottom))
            .setInsets(WindowInsetsCompat.Type.ime(), Insets.of(0, 0, 0, keyboard))
            .setVisible(WindowInsetsCompat.Type.ime(), keyboard > 0)
            .build()

    private fun withWebView(test: (Activity, WebView, SystemBars) -> Unit) {
        Robolectric.buildActivity(Activity::class.java).setup().use { controller ->
            val activity = controller.get()
            val webView = WebView(activity)
            activity.setContentView(webView)
            val bars = SystemBars(activity, webView)
            bars.install()
            test(activity, webView, bars)
        }
    }

    @Test
    fun theBarInsetsReachThePageAsCssVariables() = withWebView { _, webView, bars ->
        // xxhdpi: three device pixels to a CSS pixel.
        ViewCompat.dispatchApplyWindowInsets(webView, insets(top = 72, bottom = 48))

        val script = shadowOf(webView).lastEvaluatedJavascript
        assertTrue(script, script.contains("'--sat','24px'"))
        assertTrue(script, script.contains("'--sab','16px'"))
        assertTrue(script, script.contains("'--sal','0px'"))
        assertEquals("""{"top":24,"right":0,"bottom":16,"left":0}""", bars.insets())
        assertEquals(0, (webView.layoutParams as ViewGroup.MarginLayoutParams).bottomMargin)
    }

    @Test
    fun theKeyboardLiftsTheWebviewAndClearsTheBottomInset() = withWebView { _, webView, bars ->
        ViewCompat.dispatchApplyWindowInsets(webView, insets(top = 72, bottom = 48))
        ViewCompat.dispatchApplyWindowInsets(webView, insets(top = 72, bottom = 48, keyboard = 900))

        assertEquals(900, (webView.layoutParams as ViewGroup.MarginLayoutParams).bottomMargin)
        assertEquals("""{"top":24,"right":0,"bottom":0,"left":0}""", bars.insets())
        assertTrue(shadowOf(webView).lastEvaluatedJavascript.contains("'--sab','0px'"))

        // The keyboard closes: the webview reaches the bottom again, padded for the gesture bar.
        ViewCompat.dispatchApplyWindowInsets(webView, insets(top = 72, bottom = 48))
        assertEquals(0, (webView.layoutParams as ViewGroup.MarginLayoutParams).bottomMargin)
        assertEquals("""{"top":24,"right":0,"bottom":16,"left":0}""", bars.insets())
    }

    @Test
    fun theStatusBarIconsFollowThePageTheme() = withWebView { activity, _, bars ->
        val controller = WindowCompat.getInsetsController(activity.window, activity.window.decorView)
        bars.applyTheme(dark = true)
        assertFalse(controller.isAppearanceLightStatusBars)
        bars.applyTheme(dark = false)
        assertTrue(controller.isAppearanceLightStatusBars)
    }
}
