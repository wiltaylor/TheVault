package dev.wiltaylor.thevault.device

import android.app.Activity
import android.os.Bundle
import android.widget.Toast
import java.util.concurrent.Executors

/**
 * The share target: other apps send text, links, images and files here, and each share runs the
 * `share_received` handler, which by default makes a fleeting note in the local vault (user
 * story 6). It shows no window of its own, only a toast, and works with no connection; the note
 * syncs with the next round.
 */
class ShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val intent = intent
        val app = applicationContext
        worker.execute {
            val message = try {
                val outcome = ShareImporter.import(app, intent)
                when {
                    outcome == null -> "Nothing to save"
                    outcome.getString("status") == "done" -> "Saved to TheVault inbox"
                    // The event stays queued and runs again with the next pass.
                    else -> "TheVault will try this share again: ${outcome.optString("message")}"
                }
            } catch (e: Exception) {
                "TheVault could not save this: ${e.message}"
            }
            runOnUiThread {
                Toast.makeText(app, message, Toast.LENGTH_SHORT).show()
                finish()
            }
        }
    }

    companion object {
        /** One share at a time, off the main thread. */
        private val worker = Executors.newSingleThreadExecutor()
    }
}
