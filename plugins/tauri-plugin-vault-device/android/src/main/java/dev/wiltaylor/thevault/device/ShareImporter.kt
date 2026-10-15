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
            val mime = context.contentResolver.getType(uri) ?: ""
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
