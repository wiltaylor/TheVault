package dev.wiltaylor.thevault.device

import android.os.Build
import android.os.Bundle
import android.text.InputType
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.activity.ComponentActivity
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import java.util.concurrent.Executors
import org.json.JSONObject

/**
 * First-run setup (D2, D10 onboarding): pair with the home server by its address and the vault
 * password, or by scanning the pairing QR code that Settings > Devices shows on the web app. The
 * token the server returns goes into the Android Keystore ([PairingStore]); then the first sync
 * starts and background sync is scheduled.
 */
class OnboardingActivity : ComponentActivity() {
    lateinit var serverUrl: EditText
    lateinit var password: EditText
    lateinit var deviceName: EditText
    lateinit var cacheMb: EditText
    lateinit var pairButton: Button
    lateinit var scanButton: Button
    lateinit var message: TextView

    private val scan = registerForActivityResult(ScanContract()) { result ->
        val text = result.contents ?: return@registerForActivityResult
        val qr = PairingQr.parse(text)
        if (qr == null) {
            message.text = "That QR code is not a TheVault pairing code."
        } else {
            serverUrl.setText(qr.serverUrl)
            pair(qr.serverUrl, null, qr.code)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val pad = (16 * resources.displayMetrics.density).toInt()
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad * 2, pad, pad)
        }
        fun field(hint: String, type: Int): EditText = EditText(this).apply {
            this.hint = hint
            inputType = type
            column.addView(this, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
        }
        column.addView(TextView(this).apply {
            text = "Connect TheVault to your server"
            textSize = 22f
        })
        column.addView(TextView(this).apply {
            text = "Scan the pairing code from Settings > Devices on the web app, or enter the server address and the vault password."
        })
        scanButton = Button(this).apply { text = "Scan pairing QR code" }
        column.addView(scanButton)
        serverUrl = field("Server address, https://…", InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        password = field("Vault password", InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD)
        deviceName = field("Device name", InputType.TYPE_CLASS_TEXT)
        deviceName.setText(Build.MODEL ?: "Android")
        cacheMb = field("Offline file cache, MB", InputType.TYPE_CLASS_NUMBER)
        cacheMb.setText(PairingStore(this).cacheMb.toString())
        pairButton = Button(this).apply { text = "Pair" }
        column.addView(pairButton)
        message = TextView(this)
        column.addView(message)
        setContentView(ScrollView(this).apply { addView(column) })

        scanButton.setOnClickListener {
            scan.launch(
                ScanOptions()
                    .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                    .setPrompt("Scan the pairing code from Settings > Devices")
                    .setBeepEnabled(false)
                    .setOrientationLocked(false),
            )
        }
        pairButton.setOnClickListener {
            val pw = password.text.toString()
            if (pw.isEmpty()) {
                message.text = "Enter the vault password, or scan a pairing code."
            } else {
                pair(serverUrl.text.toString(), pw, null)
            }
        }
    }

    /** Pairs off the main thread, then stores the pairing, starts syncing and closes. */
    fun pair(url: String, pw: String?, code: String?) {
        val name = deviceName.text.toString()
        val cache = cacheMb.text.toString().toLongOrNull() ?: PairingStore.DEFAULT_CACHE_MB
        busy(true)
        message.text = "Pairing…"
        worker.execute {
            val error = try {
                val paired = JSONObject(VaultCore.pair(url, pw, code, name))
                PairingStore(applicationContext).save(paired, cache)
                Vault.open(applicationContext)
                VaultCore.kick()
                SyncWorker.schedule(applicationContext)
                null
            } catch (e: Exception) {
                e.message ?: "pairing failed"
            }
            runOnUiThread {
                busy(false)
                if (error == null) {
                    setResult(RESULT_OK)
                    finish()
                } else {
                    message.text = error
                }
            }
        }
    }

    private fun busy(on: Boolean) {
        pairButton.isEnabled = !on
        scanButton.isEnabled = !on
    }

    companion object {
        private val worker = Executors.newSingleThreadExecutor()
    }
}
