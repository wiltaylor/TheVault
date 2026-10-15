package dev.wiltaylor.thevault.device

import android.app.Notification
import android.app.Person
import android.content.Context
import android.content.Intent
import android.os.Process
import android.provider.Telephony.Sms
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import androidx.test.core.app.ApplicationProvider
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * S5: Google Messages notification parsing into `rcs_received` (D13). The sample notifications
 * produce their expected events; a group chat is one conversation; messages the Telephony
 * provider already holds as SMS or MMS are dropped; other apps' notifications are ignored.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class RcsCaptureTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private lateinit var phone: FakePhone

    @Before
    fun setUp() {
        phone = FakePhone(context)
        phone.install()
    }

    @After
    fun tearDown() {
        Device.reset()
    }

    private fun fixture(name: String) = RcsFixture.load(context, name)

    private fun RcsCapture.capture(f: RcsFixture) = capture(f.packageName, f.key, f.notification)

    private fun events() = phone.sink.events.onEach { assertEquals("rcs_received", it.event) }.map { it.data }

    private fun assertEvent(expect: JSONObject, actual: JSONObject, where: String) {
        for (k in listOf("device_ref", "thread_ref", "sender", "body")) {
            assertEquals("$where $k", expect.getString(k), actual.getString(k))
        }
        assertEquals("$where conversation", expect.opt("conversation").takeUnless { expect.isNull("conversation") }, actual.opt("conversation").takeUnless { actual.isNull("conversation") })
        assertEquals("$where sent", expect.getLong("sent_ms"), Payloads.parseTime(actual.getString("sent")))
        assertEquals("$where partial", true, actual.getBoolean("partial"))
    }

    @Test
    fun everyFixtureParsesToItsExpectedEvents() {
        for (name in RcsFixture.ALL) {
            val f = fixture(name)
            val parsed = GoogleMessages.parse(f.packageName, f.key, f.notification).map { it.payload() }
            assertEquals("$name: events", f.expect.size, parsed.size)
            f.expect.zip(parsed).forEachIndexed { i, (e, a) -> assertEvent(e, a, "$name[$i]") }
        }
    }

    @Test
    fun fixtureNotificationsProduceTheirEventsThroughCapture() {
        val rcs = phone.rcs()
        for (name in listOf("one-to-one", "group")) rcs.capture(fixture(name))

        val expect = fixture("one-to-one").expect + fixture("group").expect
        val actual = events()
        assertEquals(expect.size, actual.size)
        expect.zip(actual).forEachIndexed { i, (e, a) -> assertEvent(e, a, "event $i") }
        assertEquals(2, phone.sink.flushes)
    }

    @Test
    fun aGroupChatIsOneConversationAndARepostCapturesOnlyItsNewMessage() {
        val rcs = phone.rcs()
        assertEquals(2, rcs.capture(fixture("group")))
        assertEquals(1, rcs.capture(fixture("group-repost")))
        // Reposted again, as when the notification updates without a new message.
        assertEquals(0, rcs.capture(fixture("group-repost")))

        val captured = events()
        assertEquals(listOf("Who's in for Saturday?", "Me!", "Meet at 8 at the gym"), captured.map { it.getString("body") })
        assertEquals(setOf("2210"), captured.map { it.getString("thread_ref") }.toSet())
        assertEquals(setOf("Climbing crew"), captured.map { it.getString("conversation") }.toSet())
        assertEquals(listOf("Sam Lee", "Priya Shah", "Sam Lee"), captured.map { it.getString("sender") })
    }

    @Test
    fun messagesAlreadySeenStayCapturedOnceAcrossRestarts() {
        assertEquals(2, phone.rcs().capture(fixture("one-to-one")))
        // A new process: the seen list survives in preferences.
        assertEquals(0, phone.rcs().capture(fixture("one-to-one")))
        assertEquals(2, events().size)
    }

    @Test
    fun aMessageTheProviderHoldsAsSmsIsDropped() {
        val f = fixture("sms")
        val at = f.expect.single().getLong("sent_ms")
        phone.sms(1, "0412 345 678", "are you there?", Sms.MESSAGE_TYPE_INBOX, date = at + 2_000)

        assertEquals(0, phone.rcs().capture(f))
        assertTrue(events().isEmpty())
        assertEquals(0, phone.sink.flushes)
    }

    @Test
    fun aMessageTheProviderHoldsAsMmsIsDropped() {
        val f = fixture("sms")
        val at = f.expect.single().getLong("sent_ms")
        phone.mms(4, "0412 345 678", listOf(FakePhone.SELF), dateSeconds = at / 1000)
        phone.part(40, 4, "application/smil")
        phone.part(41, 4, "text/plain", text = "are you there?")

        assertEquals(0, phone.rcs().capture(f))
        assertTrue(events().isEmpty())
    }

    @Test
    fun theSameTextAtAnotherTimeOrASentSmsIsNotADuplicate() {
        val f = fixture("sms")
        val at = f.expect.single().getLong("sent_ms")
        // Received an hour earlier, and sent by this phone at the same moment: neither is this message.
        phone.sms(1, "0412 345 678", "are you there?", Sms.MESSAGE_TYPE_INBOX, date = at - 3_600_000)
        phone.sms(2, "0412 345 678", "are you there?", Sms.MESSAGE_TYPE_SENT, date = at)
        phone.sms(3, "0412 345 678", "something else", Sms.MESSAGE_TYPE_INBOX, date = at)

        assertEquals(1, phone.rcs().capture(f))
        assertEquals("are you there?", events().single().getString("body"))
    }

    @Test
    fun otherAppsAndNonConversationNotificationsAreIgnored() {
        val rcs = phone.rcs()
        for (name in listOf("other-app", "summary", "not-a-conversation")) {
            assertEquals(name, 0, rcs.capture(fixture(name)))
        }
        assertTrue(events().isEmpty())
    }

    @Test
    fun aNotificationBuiltByThePlatformMessagingStyleParses() {
        val me = Person.Builder().setName("You").setKey("self").build()
        val sam = Person.Builder().setName("Sam Lee").setKey("lookup:sam").build()
        val style = Notification.MessagingStyle(me)
            .setConversationTitle("Climbing crew")
            .setGroupConversation(true)
            .addMessage("Who's in?", 1_792_454_400_000, sam)
            .addMessage("Me", 1_792_454_410_000, me)
            .addMessage("Sent from the notification", 1_792_454_420_000, null as Person?)
        val n = Notification.Builder(context, "messages")
            .setSmallIcon(android.R.drawable.sym_action_chat)
            .setShortcutId("2210")
            .setStyle(style)
            .build()

        val parsed = GoogleMessages.parse(GoogleMessages.PACKAGE, "k", n)

        assertEquals(1, parsed.size)
        assertEquals(GoogleMessages.Incoming("rcs:k:1792454400000", "2210", "Sam Lee", "Climbing crew", "Who's in?", 1_792_454_400_000), parsed.single())
    }

    @Test
    fun theListenerIsDeclaredBehindTheBindPermission() {
        val services = context.packageManager.queryIntentServices(
            Intent(NotificationListenerService.SERVICE_INTERFACE).setPackage(context.packageName),
            0,
        )
        val listener = services.single { it.serviceInfo.name == RcsListenerService::class.java.name }
        assertEquals(android.Manifest.permission.BIND_NOTIFICATION_LISTENER_SERVICE, listener.serviceInfo.permission)
    }

    @Test
    fun theListenerCapturesGoogleMessagesAndIgnoresOtherApps() {
        val service = Robolectric.setupService(RcsListenerService::class.java)
        val group = fixture("group")
        val other = fixture("other-app")

        service.onNotificationPosted(posted(other.packageName, 5, other.notification))
        service.onNotificationPosted(posted(group.packageName, 2210, group.notification))

        val captured = events()
        assertEquals(listOf("Who's in for Saturday?", "Me!"), captured.map { it.getString("body") })
        assertEquals(setOf("2210"), captured.map { it.getString("thread_ref") }.toSet())
        assertEquals(1, phone.sink.flushes)
    }

    private fun posted(pkg: String, id: Int, n: Notification): StatusBarNotification =
        @Suppress("DEPRECATION")
        StatusBarNotification(pkg, pkg, id, null, 10187, 0, 0, n, Process.myUserHandle(), 0L)
}
