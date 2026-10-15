package com.google.android.apps.messaging;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Person;
import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.provider.ContactsContract;
import java.util.List;

/**
 * TEST ONLY. Posts conversation notifications the way Google Messages does: one MessagingStyle
 * notification per conversation, its shortcut id the conversation id, each sender a Person, and
 * the user's own Person as the style's user.
 */
public final class Notifier {
    static final String TAG = "FakeMessages";
    private static final String CHANNEL = "conversations";

    /** One message in a conversation notification. */
    public static final class Line {
        final String sender;
        final String key;
        final String text;
        final long time;

        Line(String sender, String key, String text, long time) {
            this.sender = sender;
            this.key = key;
            this.text = text;
            this.time = time;
        }
    }

    private Notifier() {}

    /** Posts (or reposts) conversation [conversation] holding [lines]; a group when [groupTitle] is set. */
    static void post(Context context, String conversation, String groupTitle, Line... lines) {
        post(context, conversation, groupTitle, java.util.Arrays.asList(lines));
    }

    static void post(Context context, String conversation, String groupTitle, List<Line> lines) {
        NotificationManager nm = context.getSystemService(NotificationManager.class);
        nm.createNotificationChannel(new NotificationChannel(CHANNEL, "Conversations", NotificationManager.IMPORTANCE_HIGH));
        Person me = new Person.Builder().setName("You").setKey("self").build();
        Notification.MessagingStyle style = new Notification.MessagingStyle(me);
        if (groupTitle != null) {
            style.setConversationTitle(groupTitle);
            style.setGroupConversation(true);
        }
        for (Line l : lines) {
            Person p = new Person.Builder().setName(l.sender).setKey(l.key).build();
            style.addMessage(new Notification.MessagingStyle.Message(l.text, l.time, p));
        }
        Notification n = new Notification.Builder(context, CHANNEL)
            .setSmallIcon(android.R.drawable.sym_action_chat)
            .setStyle(style)
            .setShortcutId(conversation)
            .setCategory(Notification.CATEGORY_MESSAGE)
            .build();
        nm.notify(conversation, 1, n);
    }

    /** The contact name for [number], else the number. */
    static String nameOf(Context context, String number) {
        try (Cursor c = context.getContentResolver().query(
                Uri.withAppendedPath(ContactsContract.PhoneLookup.CONTENT_FILTER_URI, Uri.encode(number)),
                new String[] {ContactsContract.PhoneLookup.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst()) return c.getString(0);
        } catch (Exception e) {
            // No contacts permission: the number it is.
        }
        return number;
    }
}
