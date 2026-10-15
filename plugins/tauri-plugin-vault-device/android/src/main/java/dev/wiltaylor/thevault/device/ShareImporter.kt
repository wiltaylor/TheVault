package dev.wiltaylor.thevault.device

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import java.io.File
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject

/**
 * Turns a share intent into a fleeting note in the local vault, with no connection needed
 * (D19, note 01). Each shared `content://` file is copied natively into the app's own cache
 * folder, since the sharing app's grant ends with the intent; the core stores the copies as
 * assets and removes them.
 */
object ShareImporter {
    /** What [intent] shares, or null when it shares nothing this app takes. */
    fun read(context: Context, intent: Intent): JSONObject? {
        val action = intent.action
        if (action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE) return null
        val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString() ?: ""
        val subject = intent.getCharSequenceExtra(Intent.EXTRA_SUBJECT)?.toString() ?: ""
        val files = JSONArray()
        val folder = File(context.cacheDir, "share").apply { mkdirs() }
        for (uri in streams(intent)) {
            val name = displayName(context, uri)
            val copy = File(folder, UUID.randomUUID().toString())
            val input = context.contentResolver.openInputStream(uri)
                ?: throw IllegalStateException("cannot read shared file $name")
            input.use { src -> copy.outputStream().use { dst -> src.copyTo(dst) } }
            files.put(JSONObject().put("path", copy.path).put("filename", name))
        }
        if (text.isBlank() && subject.isBlank() && files.length() == 0) return null
        return JSONObject().put("text", text).put("subject", subject).put("files", files)
    }

    /** Stores what [intent] shares as a fleeting note and answers its id, or null for nothing. */
    fun import(context: Context, intent: Intent): String? {
        val shared = read(context, intent) ?: return null
        Vault.open(context)
        return VaultCore.share(shared.toString())
    }

    private fun streams(intent: Intent): List<Uri> {
        if (intent.action == Intent.ACTION_SEND_MULTIPLE) {
            val list = if (Build.VERSION.SDK_INT >= 33) {
                intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
            } else {
                @Suppress("DEPRECATION")
                intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)
            }
            return list.orEmpty().filterNotNull()
        }
        val one = if (Build.VERSION.SDK_INT >= 33) {
            intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            @Suppress("DEPRECATION")
            intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)
        }
        return listOfNotNull(one)
    }

    /** The file's display name from its provider, else the last segment of its URI. */
    private fun displayName(context: Context, uri: Uri): String {
        try {
            context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) {
                    val i = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (i >= 0) c.getString(i)?.takeIf { it.isNotBlank() }?.let { return it }
                }
            }
        } catch (e: Exception) {
            // Fall back to the URI.
        }
        return uri.lastPathSegment?.substringAfterLast('/')?.takeIf { it.isNotBlank() } ?: "shared"
    }
}
