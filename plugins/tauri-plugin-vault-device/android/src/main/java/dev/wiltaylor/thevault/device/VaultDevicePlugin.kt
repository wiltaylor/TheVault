package dev.wiltaylor.thevault.device

import android.app.Activity
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Plugin

/**
 * TheVault's device plugin (note 01). It has no commands yet; the SMS, call log, contacts,
 * notification listener, UnifiedPush and WorkManager code lands here.
 */
@TauriPlugin
class VaultDevicePlugin(activity: Activity) : Plugin(activity)
