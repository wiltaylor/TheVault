package dev.wiltaylor.thevault.device

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.core.content.IntentCompat
import java.io.File
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject

/**
 * Hands a share intent to the core as a `share_received` device event, with no connection
 * needed (D19, note 01). Each shared `content://` file is copied natively into the app's own
 * cache folder, since the sharing app's grant ends with the intent; the core stores the copies as
 * assets, passes them to the handler as the event's attachments, and removes them. The device
 * file's handler, by default `scripts/device_share.ws`, decides the note: a fleeting note in the
 * inbox unless the user changed it.
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
            val mime = mimeOf(context, uri, intent)
            files.put(JSONObject().put("path", copy.path).put("filename", name).put("mime", mime))
        }
        if (text.isBlank() && subject.isBlank() && files.length() == 0) return null
        return JSONObject().put("text", text).put("subject", subject).put("files", files)
    }

    /**
     * Runs what [intent] shares through the device runtime and answers what became of it,
     * `{event, status, message, notes}` (see [VaultCore.share]), or null when it shares nothing.
     */
    fun import(context: Context, intent: Intent): JSONObject? {
        val shared = read(context, intent) ?: return null
        Device.ensureCore(context)
        val outcome = JSONObject(VaultCore.share(shared.toString()))
        // The share target closes at once, and the process may then be frozen: WorkManager
        // pushes the note rather than the core's foreground loop.
        SyncWorker.soon(context)
        return outcome
    }

    /**
     * Every file [intent] shares, each once, in the order it lists them: `EXTRA_STREAM`, one URI
     * or a list of them whichever the action, then the URIs of its `ClipData`. Some apps hand
     * their files over in the `ClipData` alone, often beside a line of text, and reading only
     * `EXTRA_STREAM` kept the text and dropped the files (capture K4). `IntentCompat` reads the
     * extra untyped on Android 13, whose typed getters can fail on a parcelled intent.
     */
    private fun streams(intent: Intent): List<Uri> {
        val out = LinkedHashSet<Uri>()
        // Each read answers null, or throws on some versions, when the extra is the other form.
        runCatching { IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java) }
            .getOrNull()?.let { out.add(it) }
        runCatching { IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java) }
            .getOrNull()?.filterNotNull()?.let { out.addAll(it) }
        intent.clipData?.let { clip ->
            for (i in 0 until clip.itemCount) clip.getItemAt(i).uri?.let { out.add(it) }
        }
        return out.toList()
    }

    /**
     * The media type of a shared file: its provider's, else the share's own type when that names
     * one type, such as `image/jpeg` and not a wildcard, else none, and the core guesses it from
     * the name.
     */
    private fun mimeOf(context: Context, uri: Uri, intent: Intent): String {
        val given = try {
            context.contentResolver.getType(uri)
        } catch (e: Exception) {
            null
        }
        if (!given.isNullOrBlank()) return given
        val shared = intent.type ?: return ""
        return if (shared.contains('/') && !shared.endsWith("/*")) shared else ""
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
