package dev.wiltaylor.thevault.device

import android.app.Activity
import android.view.ViewGroup
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat

/**
 * Keeps the web client clear of the system bars and the keyboard (ticket 79). Android 15 draws
 * apps edge to edge, so the webview fills the window under the status bar and the gesture bar;
 * the webview only sometimes reports those as `env(safe-area-inset-*)`. This hands the page the
 * bar insets as the CSS variables `--sat`, `--sar`, `--sab` and `--sal` (CSS pixels) on
 * `<html>`, which the app shell pads by. The keyboard is kept out by lifting the webview's
 * bottom edge above it, as `adjustResize` did before edge to edge, and the bottom bar inset is
 * then 0, since the keyboard covers it.
 *
 * The page reads the current insets from `VaultSystemBars.insets()` when it starts, and is sent
 * each change after. It reports its theme through `VaultSystemBars.setDark()`, so the status
 * bar's icons stay readable: light icons on the dark theme, dark icons on the light one.
 */
class SystemBars(private val activity: Activity, private val webView: WebView) {
    /** The latest insets, in CSS pixels. */
    @Volatile
    var current: Css = Css(0, 0, 0, 0)
        private set

    /** The bar insets on each side, in CSS pixels. */
    data class Css(val top: Int, val right: Int, val bottom: Int, val left: Int) {
        /** `{"top":…,"right":…,"bottom":…,"left":…}`, for the page. */
        fun json(): String = """{"top":$top,"right":$right,"bottom":$bottom,"left":$left}"""

        /** A script that sets the page's inset variables. */
        fun script(): String =
            "(function(s){s.setProperty('--sat','${top}px');s.setProperty('--sar','${right}px');" +
                "s.setProperty('--sab','${bottom}px');s.setProperty('--sal','${left}px');})" +
                "(document.documentElement.style);"
    }

    /** Listens for insets on the webview and offers the page [insets] and [setDark]. */
    fun install() {
        webView.addJavascriptInterface(this, JS_NAME)
        ViewCompat.setOnApplyWindowInsetsListener(webView) { view, insets ->
            ViewCompat.onApplyWindowInsets(view, apply(insets))
        }
        ViewCompat.requestApplyInsets(webView)
    }

    /**
     * Applies one round of window insets: the keyboard's to the webview, the bars' to the page.
     * Returns the insets left for the webview's own handling: with the webview lifted above the
     * keyboard, neither the keyboard nor the bottom bar overlaps it any more.
     */
    fun apply(insets: WindowInsetsCompat): WindowInsetsCompat {
        val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
        val keyboard = insets.getInsets(WindowInsetsCompat.Type.ime()).bottom
        // Above the keyboard the webview's bottom edge is clear of the gesture bar already.
        val lift = if (keyboard > bars.bottom) keyboard else 0
        (webView.layoutParams as? ViewGroup.MarginLayoutParams)?.let { params ->
            if (params.bottomMargin != lift) {
                params.bottomMargin = lift
                webView.layoutParams = params
            }
        }
        val density = webView.resources.displayMetrics.density
        fun css(px: Int) = Math.round(px / density)
        val next = Css(css(bars.top), css(bars.right), if (lift > 0) 0 else css(bars.bottom), css(bars.left))
        if (next != current) {
            current = next
            webView.evaluateJavascript(next.script(), null)
        }
        if (lift == 0) return insets
        val types = WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
        return WindowInsetsCompat.Builder(insets)
            .setInsets(types, Insets.of(bars.left, bars.top, bars.right, 0))
            .setInsets(WindowInsetsCompat.Type.ime(), Insets.NONE)
            .build()
    }

    /** The current insets as JSON, read by the page when it starts. */
    @JavascriptInterface
    fun insets(): String = current.json()

    /** The page's theme: light status bar icons on a dark page, dark ones on a light page. */
    @JavascriptInterface
    fun setDark(dark: Boolean) {
        activity.runOnUiThread { applyTheme(dark) }
    }

    /** Sets the bars' icon colour for a [dark] or light page. */
    fun applyTheme(dark: Boolean) {
        val controller = WindowCompat.getInsetsController(activity.window, activity.window.decorView)
        controller.isAppearanceLightStatusBars = !dark
        controller.isAppearanceLightNavigationBars = !dark
    }

    companion object {
        /** The name the page reaches this by: `window.VaultSystemBars`. */
        const val JS_NAME = "VaultSystemBars"
    }
}
