package dev.wiltaylor.thevault.device

import android.Manifest
import android.app.Application
import android.content.pm.PackageManager
import android.provider.Settings
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.TextView
import androidx.test.core.app.ApplicationProvider
import androidx.work.WorkManager
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
 * S5: onboarding's capture setup asks for each permission in turn, including notification access
 * for RCS with the Allow restricted settings steps, then offers the import, off by default.
 */
@RunWith(RobolectricTestRunner::class)
class CaptureSetupActivityTest {
    private val app: Application = ApplicationProvider.getApplicationContext()

    @Before
    fun setUp() {
        WorkManagerTestInitHelper.initializeTestWorkManager(app)
        FakePhone(app).install()
    }

    @After
    fun tearDown() {
        CaptureObservers.reset()
        Device.reset()
    }

    private fun View.all(): List<View> =
        if (this is ViewGroup) listOf(this) + (0 until childCount).flatMap { getChildAt(it).all() } else listOf(this)

    private fun View.texts() = all().filterIsInstance<TextView>().filter { it.isShown }.map { it.text.toString() }

    private fun View.button(label: String) = all().filterIsInstance<Button>().first { it.isShown && it.text == label }

    /** What Settings > Notification access writes when the user turns TheVault on. */
    private fun enableListener() {
        Settings.Secure.putString(app.contentResolver, "enabled_notification_listeners", RcsListenerService.component(app).flattenToString())
    }

    private fun grantEverything() {
        shadowOf(app).grantPermissions(*CaptureSetupActivity.STEPS.flatMap { it.permissions }.toTypedArray())
        enableListener()
    }

    @Test
    @Config(sdk = [35])
    fun smsRefusedOnAndroid15ShowsTheAllowRestrictedSettingsSteps() {
        Robolectric.buildActivity(CaptureSetupActivity::class.java).setup().use { controller ->
            val activity = controller.get()
            val root = activity.window.decorView
            assertTrue(root.texts().contains("Messages"))

            root.button("Allow").performClick()
            val asked = shadowOf(activity).lastRequestedPermission
            assertEquals(listOf(Manifest.permission.RECEIVE_SMS, Manifest.permission.READ_SMS), asked.requestedPermissions.toList())

            activity.onRequestPermissionsResult(
                asked.requestCode,
                asked.requestedPermissions,
                IntArray(asked.requestedPermissions.size) { PackageManager.PERMISSION_DENIED },
            )
            assertTrue(root.texts().contains(CaptureSetupActivity.RESTRICTED_GUIDANCE))
            assertTrue(root.texts().contains("Open App info"))

            // Allowed from App info, then back: the next step.
            shadowOf(app).grantPermissions(Manifest.permission.RECEIVE_SMS, Manifest.permission.READ_SMS)
            controller.pause().resume()
            assertTrue(root.texts().contains("RCS messages"))
        }
    }

    @Test
    @Config(sdk = [33])
    fun notificationAccessForRcsShowsTheRestrictedStepsAndOpensItsSettingsPage() {
        shadowOf(app).grantPermissions(Manifest.permission.RECEIVE_SMS, Manifest.permission.READ_SMS)
        Robolectric.buildActivity(CaptureSetupActivity::class.java).setup().use { controller ->
            val activity = controller.get()
            val root = activity.window.decorView
            assertTrue(root.texts().contains("RCS messages"))
            assertTrue(root.texts().contains(CaptureSetupActivity.LISTENER_GUIDANCE))
            assertTrue(root.texts().contains("Open App info"))

            root.button("Open notification access").performClick()
            val opened = shadowOf(activity).nextStartedActivity
            assertEquals(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS, opened.action)
            assertEquals(
                RcsListenerService.component(app).flattenToString(),
                opened.getStringExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME),
            )

            root.button("Open App info").performClick()
            assertEquals(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, shadowOf(activity).nextStartedActivity.action)

            // Turned on in Settings, then back: the next step.
            enableListener()
            controller.pause().resume()
            assertTrue(RcsListenerService.enabled(app))
            assertTrue(root.texts().contains("Calls"))
        }
    }

    @Test
    @Config(sdk = [33])
    fun withEveryPermissionTheImportIsOfferedOffAndFinishingStartsCaptureWithoutIt() {
        grantEverything()
        Robolectric.buildActivity(CaptureSetupActivity::class.java).setup().use { controller ->
            val activity = controller.get()
            assertTrue(activity.window.decorView.texts().contains("Earlier messages"))
            assertFalse("the import is off by default", activity.importSwitch.isChecked)

            activity.window.decorView.button("Finish").performClick()

            assertTrue(activity.isFinishing)
            assertTrue(Device.setupDone(app))
            val work = WorkManager.getInstance(app)
            assertTrue(work.getWorkInfosForUniqueWork("vault-device-import").get().isEmpty())
            assertEquals(1, work.getWorkInfosForUniqueWork("vault-device-catch-up").get().size)
        }
    }

    @Test
    @Config(sdk = [33])
    fun turningTheImportOnQueuesItOverTheChosenRange() {
        grantEverything()
        Robolectric.buildActivity(CaptureSetupActivity::class.java).setup().use { controller ->
            val activity = controller.get()
            activity.importSwitch.isChecked = true
            assertTrue(activity.window.decorView.texts().any { it.startsWith("From ") })

            activity.window.decorView.button("Finish").performClick()

            val import = WorkManager.getInstance(app).getWorkInfosForUniqueWork("vault-device-import").get()
            assertEquals(1, import.size)
        }
    }
}
