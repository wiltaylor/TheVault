package dev.wiltaylor.thevault.device

import android.app.Activity
import android.os.Build
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** S5: the plugin loads against an Android activity on the JVM, through Robolectric. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class VaultDevicePluginTest {
    @Test
    fun pluginLoadsOnTheMinimumSdk() {
        Robolectric.buildActivity(Activity::class.java).setup().use { controller ->
            val plugin = VaultDevicePlugin(controller.get())
            assertEquals(VaultDevicePlugin::class.java, plugin.javaClass)
            assertEquals(33, Build.VERSION.SDK_INT)
        }
    }
}
