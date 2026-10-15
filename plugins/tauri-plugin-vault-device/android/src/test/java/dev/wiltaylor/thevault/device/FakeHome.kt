package dev.wiltaylor.thevault.device

import java.util.Collections
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.json.JSONObject

/**
 * A home server just big enough for pairing and one empty sync round, on a free port
 * (OkHttp's MockWebServer). It records
 * each request as `METHOD /path` (query included) and its body.
 */
class FakeHome(private val protocol: Int = 1) : AutoCloseable {
    val requests: MutableList<String> = Collections.synchronizedList(mutableListOf())
    val bodies: MutableList<String> = Collections.synchronizedList(mutableListOf())
    private val server = MockWebServer()

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
