package dev.wiltaylor.thevault.device

/**
 * What the Rust core's device module asks of the phone (`crates/script-core` `DeviceData`). The
 * core calls [call] with a method such as `sms_list` and its arguments as a JSON map, and takes
 * back `{"ok": value}` or `{"err": message}`. It may call from any thread.
 */
fun interface DeviceBridge {
    fun call(method: String, args: String): String
}

/** Hears how an import of history is going: its report as JSON, after each page and each kind. */
fun interface ImportProgress {
    fun update(report: String)
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
     * Hands the core the pairing, `{server_url, token, device_id, device_name, cache_mb,
     * vault_id}` as JSON, or null when the device is not paired. The core keeps it in memory
     * only. A pairing with another vault than the local vault is a copy of moves the local vault
     * aside and opens a fresh one (ticket 143): it answers the folder the old one moved to, and
     * null otherwise.
     */
    @JvmStatic
    external fun link(link: String?): String?

    /**
     * The id of the home server's vault the local vault is a copy of, or null before pairing or
     * the first round names it (ticket 143).
     */
    @JvmStatic
    external fun vaultId(): String?

    /** Asks the foreground sync loop for a round once writes settle. */
    @JvmStatic
    external fun kick(): String?

    /**
     * Tells the core the app came to the front ([on] true) or left it. In front, a paired core
     * follows the home server's live events and runs a round soon after each change there.
     */
    @JvmStatic
    external fun foreground(on: Boolean): String?

    /**
     * Tells the core which app it runs in, `{version, build, arches, failed: {version, count,
     * message}}` as JSON, for the version gate each round asks first (D27): with a home server
     * newer than the app and an APK of it published for this phone, the round throws instead of
     * syncing and [status]'s `update` says why. Null turns the gate off. [UpdateGate] calls it.
     */
    @JvmStatic
    external fun setApp(app: String?): String?

    /** Runs one sync round now and answers its report as JSON. It blocks for the whole round. */
    @JvmStatic
    external fun sync(): String

    /**
     * Stores a share, `{text, subject, files: [{path, filename, mime}]}` as JSON: keeps the files
     * as assets, then queues a `share_received` event with them as its attachments and runs it
     * now, so the device file's handler decides the note. Answers `{event, status, message,
     * notes}` as JSON: the queued event's id, how it stands (`done`, or `queued` when the handler
     * failed and it waits for the next pass), why it failed, and the ids of the notes it made.
     * With no device runtime open ([openDevice]) or no `share_received` handler, the core writes
     * the fleeting note directly and `event` is null, so a share is never lost. The files are
     * copies the core removes once stored.
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

    /**
     * Hands the core the phone's capture health, `{ problems, ... }` as JSON, which the next sync
     * round sends to the home server with the device's settings when it changed ([CaptureHealth]).
     */
    @JvmStatic
    external fun setCaptureHealth(health: String): String?

    /**
     * `{paired, pending, last_sync_ms, last_error, live, update}` as JSON; `update` is the version
     * gate's last verdict, `{state, version, message}` with `state` `sync`, `warn` or `pause`.
     */
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

    /**
     * The device events whose handler failed on every pass since the last call, oldest first,
     * each answered once, as a JSON list of `{ id, event, at, status, attempts, message, who }`,
     * for [CaptureProblems] to announce.
     */
    @JvmStatic
    external fun newFailures(): String

    /** Every event in the queue, oldest first, with its payload, as a JSON list. */
    @JvmStatic
    external fun events(): String

    /** Stores [bytes] as an asset called [filename] in the local vault. Answers `{ asset, filename, size }` as JSON. */
    @JvmStatic
    external fun putAsset(filename: String, bytes: ByteArray): String

    /**
     * Moves the file at [path] into the local vault as an asset called [filename], without
     * reading it into memory, such as a large MMS part. The file is gone afterwards, stored or
     * not. Answers `{ asset, filename, size }` as JSON.
     */
    @JvmStatic
    external fun putAssetFile(path: String, filename: String): String

    /**
     * The bytes of asset [id], fetched from the home server first when the device does not hold
     * them, or null when neither does. It may make a network request: never on the main thread.
     */
    @JvmStatic
    external fun assetBytes(id: String): ByteArray?

    /**
     * Imports the phone's history as [request] says, `{ kinds, since, until }` as JSON: the core
     * reads each kind page by page through the device runtime's bridge (`history`), and runs each
     * message and call that has no note yet as its device event, so the device file's handlers
     * decide the notes. One whose note exists is left alone, whatever capture's cursors say.
     * [progress] hears the report as it goes. Answers the final report, `{ kinds: [{ kind, found,
     * imported, notes, existing, skipped, failed, missing_permission, error, done }], since,
     * until, done }`, as JSON. It blocks for the whole import: never call it on the main thread.
     */
    @JvmStatic
    external fun importHistory(request: String, progress: ImportProgress?): String

    /** `"true"` when the local vault holds the `message` or `call` note of [deviceRef], archived or not. */
    @JvmStatic
    external fun noted(deviceRef: String): String
}
