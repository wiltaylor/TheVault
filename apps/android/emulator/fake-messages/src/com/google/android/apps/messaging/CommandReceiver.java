package com.google.android.apps.messaging;

import android.content.BroadcastReceiver;
import android.content.ContentValues;
import android.content.Context;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.net.Uri;
import android.provider.Telephony;
import android.util.Log;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.List;

/**
 * TEST ONLY. What the emulator checks drive:
 *
 * thevault.test.RCS: post an RCS conversation notification. Extras: conversation (id), sender,
 * text, and optionally group (the group's title), sender2 and text2 (a second message from
 * another person), and time (ms, default now).
 *
 * thevault.test.MMS: insert an incoming MMS into the Telephony provider. Extras: from (number),
 * text, and image (any value: attach a generated PNG named photo.png).
 *
 * thevault.test.CONTACT: add a contact in one batch, as the Contacts app does. Extras: name,
 * phone. The result data is the new raw contact's id.
 */
public class CommandReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context context, Intent intent) {
        try {
            if ("thevault.test.RCS".equals(intent.getAction())) rcs(context, intent);
            else if ("thevault.test.MMS".equals(intent.getAction())) mms(context, intent);
            else if ("thevault.test.CONTACT".equals(intent.getAction())) {
                setResultData(contact(context, intent));
                setResultCode(1);
                return;
            }
            setResultCode(1);
        } catch (Exception e) {
            Log.e(Notifier.TAG, "command failed", e);
            setResultData(String.valueOf(e));
            setResultCode(2);
        }
    }

    private void rcs(Context context, Intent intent) {
        long time = intent.getLongExtra("time", System.currentTimeMillis());
        List<Notifier.Line> lines = new ArrayList<>();
        String sender = intent.getStringExtra("sender");
        lines.add(new Notifier.Line(sender, "rcs:" + sender, intent.getStringExtra("text"), time));
        String sender2 = intent.getStringExtra("sender2");
        if (sender2 != null) {
            lines.add(new Notifier.Line(sender2, "rcs:" + sender2, intent.getStringExtra("text2"), time + 1000));
        }
        Notifier.post(context, intent.getStringExtra("conversation"), intent.getStringExtra("group"), lines);
        Log.i(Notifier.TAG, "posted RCS notification " + intent.getStringExtra("conversation"));
    }

    private String contact(Context context, Intent intent) throws Exception {
        java.util.ArrayList<android.content.ContentProviderOperation> ops = new java.util.ArrayList<>();
        ops.add(android.content.ContentProviderOperation.newInsert(android.provider.ContactsContract.RawContacts.CONTENT_URI)
            .withValue(android.provider.ContactsContract.RawContacts.ACCOUNT_TYPE, null)
            .withValue(android.provider.ContactsContract.RawContacts.ACCOUNT_NAME, null)
            .build());
        ops.add(android.content.ContentProviderOperation.newInsert(android.provider.ContactsContract.Data.CONTENT_URI)
            .withValueBackReference(android.provider.ContactsContract.Data.RAW_CONTACT_ID, 0)
            .withValue(android.provider.ContactsContract.Data.MIMETYPE, android.provider.ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE)
            .withValue(android.provider.ContactsContract.CommonDataKinds.StructuredName.DISPLAY_NAME, intent.getStringExtra("name"))
            .build());
        ops.add(android.content.ContentProviderOperation.newInsert(android.provider.ContactsContract.Data.CONTENT_URI)
            .withValueBackReference(android.provider.ContactsContract.Data.RAW_CONTACT_ID, 0)
            .withValue(android.provider.ContactsContract.Data.MIMETYPE, android.provider.ContactsContract.CommonDataKinds.Phone.CONTENT_ITEM_TYPE)
            .withValue(android.provider.ContactsContract.CommonDataKinds.Phone.NUMBER, intent.getStringExtra("phone"))
            .withValue(android.provider.ContactsContract.CommonDataKinds.Phone.TYPE, android.provider.ContactsContract.CommonDataKinds.Phone.TYPE_MOBILE)
            .build());
        android.content.ContentProviderResult[] r = context.getContentResolver().applyBatch(android.provider.ContactsContract.AUTHORITY, ops);
        String id = r[0].uri.getLastPathSegment();
        Log.i(Notifier.TAG, "added contact " + id + " " + intent.getStringExtra("name"));
        return id;
    }

    private void mms(Context context, Intent intent) throws Exception {
        String from = intent.getStringExtra("from");
        String text = intent.getStringExtra("text");
        long now = System.currentTimeMillis();
        long thread = Telephony.Threads.getOrCreateThreadId(context, from);
        ContentValues m = new ContentValues();
        m.put(Telephony.Mms.THREAD_ID, thread);
        m.put(Telephony.Mms.DATE, now / 1000);
        m.put(Telephony.Mms.DATE_SENT, now / 1000);
        m.put(Telephony.Mms.MESSAGE_BOX, Telephony.Mms.MESSAGE_BOX_INBOX);
        m.put(Telephony.Mms.READ, 0);
        m.put(Telephony.Mms.SEEN, 0);
        m.put(Telephony.Mms.MESSAGE_TYPE, 132); // m-retrieve-conf
        m.put(Telephony.Mms.MMS_VERSION, 18);
        m.put(Telephony.Mms.CONTENT_TYPE, "application/vnd.wap.multipart.related");
        m.put(Telephony.Mms.MESSAGE_ID, "thevault-test-" + now);
        m.put(Telephony.Mms.TEXT_ONLY, intent.hasExtra("image") ? 0 : 1);
        Uri row = context.getContentResolver().insert(Telephony.Mms.Inbox.CONTENT_URI, m);
        String id = row.getLastPathSegment();

        addr(context, id, from, 137);
        addr(context, id, "insert-address-token", 151);

        ContentValues t = new ContentValues();
        t.put(Telephony.Mms.Part.MSG_ID, id);
        t.put(Telephony.Mms.Part.CONTENT_TYPE, "text/plain");
        t.put(Telephony.Mms.Part.CHARSET, 106);
        t.put(Telephony.Mms.Part.CONTENT_LOCATION, "text.txt");
        t.put(Telephony.Mms.Part.TEXT, text);
        context.getContentResolver().insert(Uri.parse("content://mms/" + id + "/part"), t);

        if (intent.hasExtra("image")) {
            ContentValues i = new ContentValues();
            i.put(Telephony.Mms.Part.MSG_ID, id);
            i.put(Telephony.Mms.Part.CONTENT_TYPE, "image/png");
            i.put(Telephony.Mms.Part.NAME, "photo.png");
            i.put(Telephony.Mms.Part.CONTENT_LOCATION, "photo.png");
            Uri part = context.getContentResolver().insert(Uri.parse("content://mms/" + id + "/part"), i);
            try (OutputStream out = context.getContentResolver().openOutputStream(part)) {
                image().compress(Bitmap.CompressFormat.PNG, 100, out);
            }
        }
        Log.i(Notifier.TAG, "inserted MMS " + row + " from " + from);
        Notifier.post(context, "thread_" + thread, null, new Notifier.Line(Notifier.nameOf(context, from), from, text, now));
    }

    private static void addr(Context context, String id, String address, int type) {
        ContentValues a = new ContentValues();
        a.put(Telephony.Mms.Addr.MSG_ID, id);
        a.put(Telephony.Mms.Addr.ADDRESS, address);
        a.put(Telephony.Mms.Addr.TYPE, type);
        a.put(Telephony.Mms.Addr.CHARSET, 106);
        context.getContentResolver().insert(Uri.parse("content://mms/" + id + "/addr"), a);
    }

    /** A 64x64 PNG: a red square on blue. */
    private static Bitmap image() {
        Bitmap b = Bitmap.createBitmap(64, 64, Bitmap.Config.ARGB_8888);
        Canvas c = new Canvas(b);
        c.drawColor(Color.BLUE);
        Paint p = new Paint();
        p.setColor(Color.RED);
        c.drawRect(16, 16, 48, 48, p);
        return b;
    }
}
