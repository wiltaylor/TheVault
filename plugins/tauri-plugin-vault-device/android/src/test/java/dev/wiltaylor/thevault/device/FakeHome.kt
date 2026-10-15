package dev.wiltaylor.thevault.device

import java.util.Collections
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.json.JSONObject

/**
 * A home server just big enough for pairing, one empty sync round, the notification list, the
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
    fun notify(id: String, title: String, body: String? = null, note: String? = null, url: String? = null) {
        notifications.add(
            0,
            JSONObject()
                .put("id", id)
                .put("created", 1_760_000_000_000L + notifications.size)
                .put("title", title)
                .put("body", body ?: JSONObject.NULL)
                .put("note", note ?: JSONObject.NULL)
                .put("url", url ?: JSONObject.NULL)
                .put("source", "test")
                .put("read", JSONObject.NULL),
        )
    }

    /** Marks a notification read, as reading it on another device does. */
    fun read(id: String) {
        synchronized(notifications) { notifications.find { it.getString("id") == id }?.put("read", 1L) }
    }

    /** This device's settings, as `GET /api/devices` lists them. */
    @Volatile
    var settings: JSONObject = JSONObject()

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
        path.startsWith("/api/sync/changes") -> 200 to """{"changes":[],"next":0,"latest":0}"""
        path == "/api/assets" -> 200 to """{"assets":[]}"""
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
