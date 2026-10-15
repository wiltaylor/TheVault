package dev.wiltaylor.thevault.device

/**
 * What the Rust core's device module asks of the phone (`crates/script-core` `DeviceData`). The
 * core calls [call] with a method such as `sms_list` and its arguments as a JSON map, and takes
 * back `{"ok": value}` or `{"err": message}`. It may call from any thread.
 */
fun interface DeviceBridge {
    fun call(method: String, args: String): String
}

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

    /**
     * The home server's unread notifications, newest first, as a JSON array of
     * `{id, created, title, body, note, url, ...}`. It makes a network request.
     */
    @JvmStatic
    external fun unreadNotifications(): String

    /**
     * Marks notification [id] read on the home server. Answers `"true"`, or `"false"` when the
     * server no longer has it. It makes a network request.
     */
    @JvmStatic
    external fun markRead(id: String): String

    /**
     * Registers the UnifiedPush [endpoint] and its WebPush keys, base64url, with the home server,
     * or clears the endpoint when it is null. It makes a network request.
     */
    @JvmStatic
    external fun setPush(endpoint: String?, p256dh: String?, auth: String?): String?

    /**
     * Fetches the commands server scripts queued for this device, runs each one the device file
     * declares and the device's settings enable, and reports every result to the home server.
     * Answers what ran, `[{id, name, status, result}]`, as JSON. It makes network requests.
     */
    @JvmStatic
    external fun runCommands(): String

    /** `{paired, pending, last_sync_ms, last_error}` as JSON. */
    @JvmStatic
    external fun status(): String

    /**
     * Opens the device runtime of [deviceId] on the open vault, whose device module reaches the
     * phone through [bridge]. Calling it again is safe; another [deviceId] reopens the runtime on
     * the same queue.
     */
    @JvmStatic
    external fun openDevice(deviceId: String, bridge: DeviceBridge): String?

    /** Stores device event [event] with its payload [data], JSON text, captured at [atMs]. Answers its queue id. */
    @JvmStatic
    external fun enqueue(event: String, data: String, atMs: Long): Long

    /** Runs every queued event's handlers. Answers the events taken, as a JSON list. */
    @JvmStatic
    external fun runPending(): String

    /** Every event in the queue, oldest first, with its payload, as a JSON list. */
    @JvmStatic
    external fun events(): String

    /** Stores [bytes] as an asset called [filename] in the local vault. Answers `{ asset, filename, size }` as JSON. */
    @JvmStatic
    external fun putAsset(filename: String, bytes: ByteArray): String
}
