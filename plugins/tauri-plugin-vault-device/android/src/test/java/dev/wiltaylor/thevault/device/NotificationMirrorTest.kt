package dev.wiltaylor.thevault.device

import android.Manifest
import android.app.Activity
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.service.notification.StatusBarNotification
import androidx.test.core.app.ApplicationProvider
import androidx.work.Configuration
import androidx.work.ListenableWorker
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.testing.SynchronousExecutor
import androidx.work.testing.TestWorkerBuilder
import androidx.work.testing.WorkManagerTestInitHelper
import com.google.crypto.tink.apps.fixed_webpush.WebPushHybridDecrypt
import java.util.Base64
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.unifiedpush.android.connector.data.PublicKeySet
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/**
 * S5: UnifiedPush messages show and cancel Android notifications by id, a swipe marks one read
 * on the server, and the fallback worker reconciles the shade with the server's list (D4, D9).
 * The Rust core is the real one, through JNI, against a fake home server on a free port.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class NotificationMirrorTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val home = FakeHome()
    private val manager = context.getSystemService(NotificationManager::class.java)

    @Before
    fun setUp() {
        PairingStore.defaultCipher = FakeCipher
        WorkManagerTestInitHelper.initializeTestWorkManager(
            context,
            Configuration.Builder().setExecutor(SynchronousExecutor()).build(),
        )
        shadowOf(context as android.app.Application).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
        PairingStore(context).save(
            JSONObject()
                .put("server_url", home.url)
                .put("device_id", FakeHome.DEVICE_ID)
                .put("device_name", "Pixel")
                .put("token", FakeHome.TOKEN),
        )
    }

    @After
    fun stop() {
        home.close()
    }

    private val service: VaultPushService by lazy {
        Robolectric.buildService(VaultPushService::class.java).create().get()
    }

    /** What the distributor hands over once the connector has decrypted it. */
    private fun push(json: String) {
        service.onMessage(PushMessage(json.toByteArray(Charsets.UTF_8), true), "default")
    }

    private fun showing(): Map<String, StatusBarNotification> =
        manager.activeNotifications.associateBy { it.tag }

    /** Runs the work queued under [name] as if its network constraint were met. */
    private fun meetConstraints(name: String) {
        val driver = WorkManagerTestInitHelper.getTestDriver(context)!!
        for (info in WorkManager.getInstance(context).getWorkInfosForUniqueWork(name).get()) {
            driver.setAllConstraintsMet(info.id)
        }
    }

    private fun runFallback(): ListenableWorker.Result =
        TestWorkerBuilder.from(context, NotificationWorker::class.java, Executors.newSingleThreadExecutor())
            .build()
            .doWork()

    @Test
    fun aCreatedPushShowsTheNotificationUnderTheServerId() {
        home.notify("01NOTIFOLD0000000000000000", "Already there before pairing")
        home.notify("01NOTIFNEW0000000000000000", "Build finished", body = "All green", note = "01NOTE00000000000000000000")
        push("""{"event":"notification.created","id":"01NOTIFNEW0000000000000000"}""")

        val shown = showing()
        assertEquals(setOf("01NOTIFNEW0000000000000000"), shown.keys)
        val n = shown.getValue("01NOTIFNEW0000000000000000")
        assertEquals(NotificationMirror.androidId("01NOTIFNEW0000000000000000"), n.id)
        assertEquals(NotificationMirror.CHANNEL, n.notification.channelId)
        assertEquals("Build finished", n.notification.extras.getString(Notification.EXTRA_TITLE))
        assertEquals("All green", n.notification.extras.getCharSequence(Notification.EXTRA_TEXT).toString())
        assertTrue(home.requests.any { it.startsWith("GET /api/notifications?unread=true") })

        // A tap opens the app at the note.
        val tap = shadowOf(n.notification.contentIntent).savedIntent
        assertEquals("01NOTIFNEW0000000000000000", tap.getStringExtra(NotificationMirror.EXTRA_ID))
        assertEquals("/notes/01NOTE00000000000000000000", tap.getStringExtra(NotificationMirror.EXTRA_PATH))
    }

    @Test
    fun readDismissAndClearPushesCancelTheirNotifications() {
        for (i in 1..3) home.notify("01NOTIF${i}00000000000000000", "Number $i")
        for (i in 1..3) push("""{"event":"notification.created","id":"01NOTIF${i}00000000000000000"}""")
        assertEquals(3, showing().size)

        push("""{"event":"notification.read","id":"01NOTIF100000000000000000"}""")
        assertEquals(setOf("01NOTIF200000000000000000", "01NOTIF300000000000000000"), showing().keys)
        push("""{"event":"notification.dismissed","id":"01NOTIF200000000000000000"}""")
        assertEquals(setOf("01NOTIF300000000000000000"), showing().keys)

        // A clear names no id: the phone reconciles with what is still unread.
        home.read("01NOTIF300000000000000000")
        push("""{"event":"notification.cleared"}""")
        assertTrue(showing().isEmpty())
        // Pushes this app does not handle change nothing.
        push("""{"event":"device.command","id":"01CMD00000000000000000000"}""")
        push("not json")
    }

    @Test
    fun aSwipeMarksTheNotificationReadOnTheServer() {
        home.notify("01NOTIFSWIPE00000000000000", "Swipe me")
        push("""{"event":"notification.created","id":"01NOTIFSWIPE00000000000000"}""")
        val n = showing().getValue("01NOTIFSWIPE00000000000000")

        // The system sends the delete intent when the user swipes the notification away.
        val swipe = shadowOf(n.notification.deleteIntent).savedIntent
        NotificationDismissReceiver().onReceive(context, swipe)
        manager.cancel(n.tag, n.id)
        meetConstraints(MarkReadWorker.uniqueName("01NOTIFSWIPE00000000000000"))
        assertTrue(home.requests.toString(), home.requests.contains("POST /api/notifications/01NOTIFSWIPE00000000000000/read"))

        // Swiped before the read reached the server: the next check does not bring it back.
        home.notifications.first().put("read", JSONObject.NULL)
        assertTrue(runFallback() is ListenableWorker.Result.Success)
        assertTrue(showing().isEmpty())
    }

    @Test
    fun aTapMarksTheNotificationRead() {
        home.notify("01NOTIFTAP0000000000000000", "Tap me", url = "/settings")
        push("""{"event":"notification.created","id":"01NOTIFTAP0000000000000000"}""")
        val tap = shadowOf(showing().getValue("01NOTIFTAP0000000000000000").notification.contentIntent).savedIntent
        assertEquals("/settings", tap.getStringExtra(NotificationMirror.EXTRA_PATH))
        Robolectric.buildActivity(Activity::class.java).setup().use { controller ->
            VaultDevicePlugin(controller.get()).onNewIntent(Intent(tap))
        }
        meetConstraints(MarkReadWorker.uniqueName("01NOTIFTAP0000000000000000"))
        assertTrue(home.requests.contains("POST /api/notifications/01NOTIFTAP0000000000000000/read"))
    }

    @Test
    fun theFallbackWorkerReconcilesTheShadeWithTheServerList() {
        home.notify("01NOTIFA000000000000000000", "Read elsewhere")
        home.notify("01NOTIFB000000000000000000", "Still unread")
        push("""{"event":"notification.created","id":"01NOTIFA000000000000000000"}""")
        push("""{"event":"notification.created","id":"01NOTIFB000000000000000000"}""")
        assertEquals(2, showing().size)

        // With no push reaching the phone: one read on the web app, one new since.
        home.read("01NOTIFA000000000000000000")
        home.notify("01NOTIFC000000000000000000", "Arrived while no push came")
        assertTrue(runFallback() is ListenableWorker.Result.Success)
        assertEquals(setOf("01NOTIFB000000000000000000", "01NOTIFC000000000000000000"), showing().keys)
        assertEquals(
            NotificationMirror.androidId("01NOTIFC000000000000000000"),
            showing().getValue("01NOTIFC000000000000000000").id,
        )

        // An unreachable server is retried later and leaves the shade alone.
        home.close()
        assertTrue(runFallback() is ListenableWorker.Result.Retry)
        assertEquals(2, showing().size)
    }

    @Test
    fun withNoDistributorTheCheckRunsEveryFifteenMinutes() {
        Robolectric.buildActivity(Activity::class.java).setup().use { controller ->
            PushRegistration.start(controller.get())
        }
        val infos = WorkManager.getInstance(context).getWorkInfosForUniqueWork(NotificationWorker.FALLBACK).get()
        assertEquals(1, infos.size)
        assertEquals(WorkInfo.State.ENQUEUED, infos[0].state)
        assertEquals(TimeUnit.MINUTES.toMillis(15), infos[0].periodicityInfo!!.repeatIntervalMillis)
    }

    @Test
    fun aNewEndpointIsRegisteredAndStopsTheFallback() {
        NotificationWorker.schedule(context)
        val keys = PublicKeySet("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4", "BTBZMqHH6r4Tts7J_aSIgg")
        service.onNewEndpoint(PushEndpoint("https://push.example.net/up/abc", keys, false), "default")
        meetConstraints(PushEndpointWorker.UNIQUE_NAME)

        val i = home.requests.indexOf("PUT /api/devices/${FakeHome.DEVICE_ID}/push")
        assertTrue(home.requests.toString(), i >= 0)
        val body = JSONObject(home.bodies[i])
        assertEquals("https://push.example.net/up/abc", body.getString("endpoint"))
        assertEquals(keys.pubKey, body.getString("p256dh"))
        assertEquals(keys.auth, body.getString("auth"))
        val fallback = WorkManager.getInstance(context).getWorkInfosForUniqueWork(NotificationWorker.FALLBACK).get()
        assertTrue(fallback.all { it.state == WorkInfo.State.CANCELLED })

        // Unregistered by the distributor: the server forgets the endpoint and the check resumes.
        service.onUnregistered("default")
        meetConstraints(PushEndpointWorker.UNIQUE_NAME)
        assertTrue(JSONObject(home.bodies.last()).isNull("endpoint"))
        val resumed = WorkManager.getInstance(context).getWorkInfosForUniqueWork(NotificationWorker.FALLBACK).get()
        assertTrue(resumed.any { it.state == WorkInfo.State.ENQUEUED })
    }

    @Test
    fun aMessageTheConnectorCouldNotDecryptIsIgnored() {
        home.notify("01NOTIFX000000000000000000", "Never shown")
        service.onMessage(PushMessage("""{"event":"notification.created","id":"01NOTIFX000000000000000000"}""".toByteArray(), false), "default")
        assertTrue(showing().isEmpty())
        assertFalse(home.requests.any { it.startsWith("GET /api/notifications") })
    }

    /**
     * The connector's WebPush decrypter opens what the server's encrypter writes: the RFC 8291
     * appendix A message, which `crates/push` encrypts to byte for byte.
     */
    @Test
    fun theConnectorDecryptsTheServersWebPushFormat() {
        fun b64(s: String) = Base64.getUrlDecoder().decode(s)
        val decrypter = WebPushHybridDecrypt.Builder()
            .withAuthSecret(b64("BTBZMqHH6r4Tts7J_aSIgg"))
            .withRecipientPublicKey(b64("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4"))
            .withRecipientPrivateKey(b64("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94"))
            .build()
        val message = b64(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN",
        )
        val plain = decrypter.decrypt(message, null)
        assertNotNull(plain)
        assertArrayEquals("When I grow up, I want to be a watermelon".toByteArray(), plain)
    }

    @Test
    fun androidIdsAreStableAndDifferPerServerId() {
        val a = NotificationMirror.androidId("01NOTIFA000000000000000000")
        assertEquals(a, NotificationMirror.androidId("01NOTIFA000000000000000000"))
        assertFalse(a == NotificationMirror.androidId("01NOTIFB000000000000000000"))
        // FNV-1a's published value for "a".
        assertEquals(0xe40c292c.toInt(), NotificationMirror.androidId("a"))
    }
}
