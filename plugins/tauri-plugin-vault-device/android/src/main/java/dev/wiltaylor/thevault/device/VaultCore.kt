package dev.wiltaylor.thevault.device

/**
 * The Rust core, reached over plain JNI so that receivers and workers can call it with no
 * webview and no Tauri runtime (D17). The functions live in the plugin crate's `src/jni.rs`; a
 * failure throws [IllegalStateException] with the core's message.
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

    /** Opens the vault folder [vault], once per process, and answers it. */
    @JvmStatic
    external fun open(vault: String): String

    /**
     * Pairs with the home server at [serverUrl] by [password] or by pairing [code], whichever is
     * not null, as a device called [name]. Answers `{server_url, device_id, device_name, token}`
     * as JSON. It makes a network request: never call it on the main thread.
     */
    @JvmStatic
    external fun pair(serverUrl: String, password: String?, code: String?, name: String): String

    /**
     * Hands the core the pairing, `{server_url, token, device_id, device_name, cache_mb}` as
     * JSON, or null when the device is not paired. The core keeps it in memory only.
     */
    @JvmStatic
    external fun link(link: String?): String?

    /** Asks the foreground sync loop for a round once writes settle. */
    @JvmStatic
    external fun kick(): String?

    /** Runs one sync round now and answers its report as JSON. It blocks for the whole round. */
    @JvmStatic
    external fun sync(): String

    /**
     * Stores a share, `{text, subject, files: [{path, filename}]}` as JSON, as a fleeting note
     * and answers its id. The files are copies the core removes once stored.
     */
    @JvmStatic
    external fun share(shared: String): String

    /** The notes waiting for the home server, as a JSON array. */
    @JvmStatic
    external fun queued(): String

    /** `{paired, pending, last_sync_ms, last_error}` as JSON. */
    @JvmStatic
    external fun status(): String
}
