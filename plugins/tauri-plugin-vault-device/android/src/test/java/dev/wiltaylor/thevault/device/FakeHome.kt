package dev.wiltaylor.thevault.device

import java.util.Collections
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.json.JSONObject

/**
 * A home server just big enough for pairing, a sync round that takes pushed notes as sent, the notification list, the
 * push endpoint, and the device's settings and commands, on a free port (OkHttp's MockWebServer). It records each request as
 * `METHOD /path` (query included) and its body.
 */
class FakeHome(private val protocol: Int = 1) : AutoCloseable {
    val requests: MutableList<String> = Collections.synchronizedList(mutableListOf())
    val bodies: MutableList<String> = Collections.synchronizedList(mutableListOf())
    private val server = MockWebServer()

    /** The server's notifications, newest first, as `GET /api/notifications` sends them. */
    val notifications: MutableList<JSONObject> = Collections.synchronizedList(mutableListOf())

    /** Adds an unread notification, newest. */
    fun notify(id: String, title: String, body: String? = null, note: String? = null, url: String? = null, source: String = "test") {
        notifications.add(
            0,
            JSONObject()
                .put("id", id)
                .put("created", 1_760_000_000_000L + notifications.size)
                .put("title", title)
                .put("body", body ?: JSONObject.NULL)
                .put("note", note ?: JSONObject.NULL)
                .put("url", url ?: JSONObject.NULL)
                .put("source", source)
                .put("read", JSONObject.NULL),
        )
    }

    /** Marks a notification read, as reading it on another device does. */
    fun read(id: String) {
        synchronized(notifications) { notifications.find { it.getString("id") == id }?.put("read", 1L) }
    }

    /** The notes the device pushed, by id, as their WCL. */
    val notes: MutableMap<String, String> = Collections.synchronizedMap(mutableMapOf())

    /** The change feed a sync round pulls, oldest first. */
    private val feed: MutableList<JSONObject> = Collections.synchronizedList(mutableListOf())

    /** Adds note [id] of [type] to the change feed, as a save on the server does, at version [version]. */
    fun saved(id: String, type: String, wcl: String, version: String = "v1") {
        synchronized(feed) {
            feed.add(
                JSONObject()
                    .put("seq", feed.size + 1L)
                    .put("at", 1_760_000_000_000L)
                    .put("kind", "note")
                    .put("id", id)
                    .put("type", type)
                    .put("version", version)
                    .put("wcl", wcl)
                    .put("deleted", false)
                    .put("trashed", false),
            )
        }
    }

    /** This device's settings, as `GET /api/devices` lists them. */
    @Volatile
    var settings: JSONObject = JSONObject()

    /** The capture health the device last reported, or null before its first report. */
    @Volatile
    var health: JSONObject? = null

    /** The commands server scripts queued for this device, oldest first, each with its `status` and `result`. */
    val commands: MutableList<JSONObject> = Collections.synchronizedList(mutableListOf())

    /** Queues a pending command, as `vault::device_command` does. */
    fun command(id: String, name: String, payload: JSONObject) {
        commands.add(
            JSONObject()
                .put("id", id)
                .put("device", DEVICE_ID)
                .put("name", name)
                .put("payload", payload)
                .put("status", "pending")
                .put("result", JSONObject.NULL),
        )
    }

    /** Command [id] as it now stands. */
    fun command(id: String): JSONObject = synchronized(commands) { commands.first { it.getString("id") == id } }

    private fun unread(): List<JSONObject> =
        synchronized(notifications) { notifications.filter { it.isNull("read") } }

    init {
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                val method = request.method ?: ""
                val path = request.path ?: ""
                val body = request.body.readUtf8()
                requests.add("$method $path")
                bodies.add(body)
                val (status, answer) = answer(method, path, body)
                return MockResponse()
                    .setResponseCode(status)
                    .setHeader("Content-Type", "application/json")
                    .setBody(answer)
            }
        }
        server.start(java.net.InetAddress.getByName("127.0.0.1"), 0)
    }

    val url: String
        get() = "http://127.0.0.1:${server.port}"

    private fun answer(method: String, path: String, body: String): Pair<Int, String> = when {
        method == "POST" && path == "/api/devices/pair" -> {
            val req = JSONObject(body)
            if (req.optString("password") == PASSWORD || req.optString("code") == CODE) {
                201 to JSONObject()
                    .put("device_id", DEVICE_ID)
                    .put("token", TOKEN)
                    .put("device", JSONObject().put("name", req.getString("name")))
                    .toString()
            } else {
                401 to """{"error":"unauthorized","message":"wrong password"}"""
            }
        }
        path == "/api/sync/info" ->
            200 to """{"protocol":$protocol,"format_version":1,"server_version":"fake"}"""
        method == "PUT" && path == "/api/devices/$DEVICE_ID/bases" -> 200 to """{"bases":{}}"""
        path.startsWith("/api/sync/changes") -> {
            val since = path.substringAfter("since=", "0").substringBefore('&').toLongOrNull() ?: 0L
            val all = synchronized(feed) { feed.toList() }
            val latest = all.size.toLong()
            val page = all.filter { it.getLong("seq") > since }
            200 to JSONObject().put("changes", org.json.JSONArray(page)).put("next", latest).put("latest", latest).toString()
        }
        path == "/api/assets" -> 200 to """{"assets":[]}"""
        method == "PUT" && path.startsWith("/api/notes/") -> {
            // A note the device pushes, stored as sent, at version 1.
            val id = path.removePrefix("/api/notes/").substringBefore('?')
            val type = path.substringAfter("type=", "")
            val wcl = JSONObject(body).getString("wcl")
            notes[id] = wcl
            201 to JSONObject().put("id", id).put("type", type).put("version", "1").put("wcl", wcl).toString()
        }
        method == "GET" && path.startsWith("/api/notifications?") -> {
            val items = org.json.JSONArray(unread())
            200 to JSONObject().put("items", items).put("next", JSONObject.NULL).put("unread", items.length()).toString()
        }
        method == "POST" && path.startsWith("/api/notifications/") && path.endsWith("/read") -> {
            val id = path.removePrefix("/api/notifications/").removeSuffix("/read")
            val found = synchronized(notifications) { notifications.find { it.getString("id") == id } }
            if (found == null) {
                404 to """{"error":"not_found","message":"no such notification"}"""
            } else {
                found.put("read", 1L)
                200 to found.toString()
            }
        }
        method == "GET" && path == "/api/devices" -> {
            val me = JSONObject().put("id", DEVICE_ID).put("name", "Pixel").put("platform", "android").put("settings", settings)
            200 to JSONObject().put("devices", org.json.JSONArray().put(me)).toString()
        }
        method == "GET" && path == "/api/devices/$DEVICE_ID/commands?status=pending" -> {
            val pending = synchronized(commands) { commands.filter { it.getString("status") == "pending" } }
            200 to JSONObject().put("commands", org.json.JSONArray(pending)).toString()
        }
        method == "POST" && path.startsWith("/api/devices/$DEVICE_ID/commands/") && path.endsWith("/result") -> {
            val id = path.removePrefix("/api/devices/$DEVICE_ID/commands/").removeSuffix("/result")
            val found = synchronized(commands) { commands.find { it.getString("id") == id } }
            val report = JSONObject(body)
            when {
                found == null -> 404 to """{"error":"not_found","message":"no such command"}"""
                found.getString("status") !in listOf("pending", "delivered") ->
                    409 to """{"error":"conflict","message":"the command is finished"}"""
                else -> {
                    found.put("status", report.getString("status"))
                    if (report.has("result")) found.put("result", report.get("result"))
                    200 to found.toString()
                }
            }
        }
        method == "PUT" && path == "/api/devices/$DEVICE_ID/health" -> {
            health = JSONObject(body)
            200 to JSONObject().put("id", DEVICE_ID).put("health", health).toString()
        }
        method == "PUT" && path == "/api/devices/$DEVICE_ID/push" ->
            200 to JSONObject().put("id", DEVICE_ID).put("push", !JSONObject(body).isNull("endpoint")).toString()
        else -> 404 to """{"error":"not_found","message":"no route"}"""
    }

    override fun close() {
        server.shutdown()
    }

    companion object {
        const val PASSWORD = "correct horse"
        const val CODE = "ABCD-1234"
        const val DEVICE_ID = "01J9Z3K8Q2W6E4R5T7Y8U9I0OP"
        const val TOKEN = "tv_device_secret"
    }
}

/** Stands in for the Android Keystore, which the JVM does not have: a reversible scramble. */
object FakeCipher : TokenCipher {
    override fun encrypt(plain: ByteArray) = plain.map { (it.toInt() xor 0x5a).toByte() }.toByteArray().reversedArray()

    override fun decrypt(sealed: ByteArray) = sealed.reversedArray().map { (it.toInt() xor 0x5a).toByte() }.toByteArray()
}
