package dev.wiltaylor.thevault.device

import android.content.Context
import java.io.File

/**
 * The local vault, as the Kotlin side opens it. The Tauri app opens the same folder,
 * `<data dir>/vault`, from Rust, and both get the process's one core.
 */
object Vault {
    fun dir(context: Context): File = File(context.dataDir, "vault")

    /**
     * Opens the vault and hands the core the stored pairing, if any. Answers whether the device
     * is paired.
     */
    fun open(context: Context): Boolean {
        VaultCore.open(dir(context).path)
        val link = PairingStore(context).link()
        VaultCore.link(link?.toString())
        return link != null
    }
}
