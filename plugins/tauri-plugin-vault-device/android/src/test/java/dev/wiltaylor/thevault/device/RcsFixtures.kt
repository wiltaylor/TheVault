package dev.wiltaylor.thevault.device

import android.app.Notification
import android.app.Person
import android.content.Context
import android.os.Bundle
import android.os.Parcelable
import org.json.JSONArray
import org.json.JSONObject

/**
 * The sample notifications under `src/test/resources/rcs/`, in the shape Google Messages posts
 * them: the posting package, the notification key and shortcut id, and the raw extras a
 * listener reads (`android.messages` as an array of message bundles, people as `Person`s). Each
 * sample lists the `rcs_received` events it should produce under `expect`.
 *
 * In the JSON, `{"$person": {"name", "key"}}` stands for an `android.app.Person`.
 */
data class RcsFixture(
    val name: String,
    val packageName: String,
    val key: String,
    val notification: Notification,
    val expect: List<JSONObject>,
) {
    companion object {
        val ALL = listOf("one-to-one", "group", "group-repost", "sms", "other-app", "summary", "not-a-conversation")

        fun load(context: Context, name: String): RcsFixture {
            val text = RcsFixture::class.java.getResourceAsStream("/rcs/$name.json")!!.bufferedReader().use { it.readText() }
            val json = JSONObject(text)
            val builder = Notification.Builder(context, "messages")
                .setSmallIcon(android.R.drawable.sym_action_chat)
            json.optString("shortcut_id").takeIf { !json.isNull("shortcut_id") && it.isNotEmpty() }?.let { builder.setShortcutId(it) }
            if (json.optBoolean("group_summary")) builder.setGroup("messages").setGroupSummary(true)
            val notification = builder.build()
            notification.extras.putAll(bundle(json.getJSONObject("extras")))
            val expect = json.getJSONArray("expect").let { a -> (0 until a.length()).map { a.getJSONObject(it) } }
            return RcsFixture(name, json.getString("package"), json.getString("key"), notification, expect)
        }

        private fun bundle(json: JSONObject): Bundle = Bundle().apply {
            for (k in json.keys()) {
                when (val v = json.get(k)) {
                    is String -> putCharSequence(k, v)
                    is Boolean -> putBoolean(k, v)
                    is Int -> putLong(k, v.toLong())
                    is Long -> putLong(k, v)
                    is JSONArray -> putParcelableArray(k, Array<Parcelable>(v.length()) { bundle(v.getJSONObject(it)) })
                    is JSONObject -> putParcelable(k, person(v.getJSONObject("\$person")))
                    else -> error("fixture value $k: $v")
                }
            }
        }

        private fun person(json: JSONObject): Person = Person.Builder()
            .setName(json.getString("name"))
            .setKey(json.optString("key").takeIf { it.isNotEmpty() })
            .build()
    }
}
