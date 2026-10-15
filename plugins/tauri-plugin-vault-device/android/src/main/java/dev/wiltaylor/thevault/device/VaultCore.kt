package dev.wiltaylor.thevault.device

/**
 * The Rust core, reached over plain JNI so that receivers and workers can call it with no
 * webview and no Tauri runtime (D17). The functions live in the plugin crate's `src/jni.rs`.
 *
 * On a phone this loads `libapp_lib.so`, the app's Rust library. The JVM tests set the
 * `vault.core.library` system property to the path of a host build instead.
 */
object VaultCore {
    init {
        val path = System.getProperty("vault.core.library")
        if (path != null) System.load(path) else System.loadLibrary("app_lib")
    }

    /** Answers the core's name and version, then [message]: a round trip into Rust and back. */
    @JvmStatic
    external fun ping(message: String): String
}
