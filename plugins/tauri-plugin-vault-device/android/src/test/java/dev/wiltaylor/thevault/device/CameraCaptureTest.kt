package dev.wiltaylor.thevault.device

import android.Manifest
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.provider.MediaStore
import androidx.test.core.app.ApplicationProvider
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * Capture K1, the app's part: a file input that accepts images and has `capture` opens the
 * camera. The webview's file chooser is wry's generated `RustWebChromeClient`, which already
 * starts `MediaStore.ACTION_IMAGE_CAPTURE` with a FileProvider output URI for such an input and
 * asks for CAMERA first, but only when `resolveActivity` finds a camera app. From Android 11 that
 * finds nothing unless the app declares the intent under `<queries>`, so the input fell back to
 * the file picker. These tests hold the declarations that chooser needs.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class CameraCaptureTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    private val android = "http://schemas.android.com/apk/res/android"

    /** The plugin's manifest, which the app merges. Gradle runs the tests in the module folder. */
    private fun manifest() = DocumentBuilderFactory.newInstance()
        .apply { isNamespaceAware = true }
        .newDocumentBuilder()
        .parse(File("src/main/AndroidManifest.xml"))

    @Test
    fun theAppMaySeeTheCameraAppsItsFileInputsStart() {
        val queries = manifest().getElementsByTagName("queries")
        assertEquals("one <queries> element", 1, queries.length)
        val actions = (0 until queries.item(0).childNodes.length)
            .map { queries.item(0).childNodes.item(it) }
            .filter { it.nodeName == "intent" }
            .flatMap { intent ->
                (0 until intent.childNodes.length).map { intent.childNodes.item(it) }
                    .filter { it.nodeName == "action" }
                    .map { it.attributes.getNamedItemNS(android, "name").nodeValue }
            }
        assertTrue(actions.toString(), MediaStore.ACTION_IMAGE_CAPTURE in actions)
        assertTrue(actions.toString(), MediaStore.ACTION_VIDEO_CAPTURE in actions)
    }

    @Test
    fun theCameraPermissionIsDeclaredSoTheChooserAsksForIt() {
        val info = context.packageManager.getPackageInfo(context.packageName, PackageManager.GET_PERMISSIONS)
        assertTrue(info.requestedPermissions.orEmpty().contains(Manifest.permission.CAMERA))
        // The camera stays optional: a phone without one still installs the app.
        val features = manifest().getElementsByTagName("uses-feature")
        val camera = (0 until features.length).map { features.item(it) }
            .single { it.attributes.getNamedItemNS(android, "name").nodeValue == "android.hardware.camera" }
        assertEquals("false", camera.attributes.getNamedItemNS(android, "required").nodeValue)
    }

    @Test
    fun theChoosersCameraIntentFindsACameraApp() {
        // The intent wry's chooser builds for a capture input, resolved as it resolves it.
        val camera = ComponentName("com.example.camera", "com.example.camera.Capture")
        val pm = shadowOf(context.packageManager)
        pm.addActivityIfNotPresent(camera)
        pm.addIntentFilterForActivity(camera, IntentFilter(MediaStore.ACTION_IMAGE_CAPTURE).apply {
            addCategory(Intent.CATEGORY_DEFAULT)
        })
        val take = Intent(MediaStore.ACTION_IMAGE_CAPTURE)
        val found = take.resolveActivity(context.packageManager)
        assertNotNull("no camera app for the capture input", found)
        assertEquals(camera, found)
    }
}
