package com.google.android.apps.messaging;

import android.content.BroadcastReceiver;
import android.content.ContentValues;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.provider.Telephony;
import android.telephony.SmsMessage;
import android.util.Log;

/** TEST ONLY. SMS_DELIVER, as the default SMS app: store the SMS, then notify as Google Messages does. */
public class SmsDeliverReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context context, Intent intent) {
        SmsMessage[] parts = Telephony.Sms.Intents.getMessagesFromIntent(intent);
        if (parts == null || parts.length == 0) return;
        StringBuilder body = new StringBuilder();
        for (SmsMessage p : parts) body.append(p.getMessageBody());
        String from = parts[0].getOriginatingAddress();
        long sent = parts[0].getTimestampMillis();
        long now = System.currentTimeMillis();
        long thread = Telephony.Threads.getOrCreateThreadId(context, from);
        ContentValues v = new ContentValues();
        v.put(Telephony.Sms.ADDRESS, from);
        v.put(Telephony.Sms.BODY, body.toString());
        v.put(Telephony.Sms.DATE, now);
        v.put(Telephony.Sms.DATE_SENT, sent);
        v.put(Telephony.Sms.READ, 0);
        v.put(Telephony.Sms.SEEN, 0);
        v.put(Telephony.Sms.TYPE, Telephony.Sms.MESSAGE_TYPE_INBOX);
        v.put(Telephony.Sms.THREAD_ID, thread);
        Uri row = context.getContentResolver().insert(Telephony.Sms.Inbox.CONTENT_URI, v);
        Log.i(Notifier.TAG, "stored SMS from " + from + " as " + row);
        Notifier.post(context, "thread_" + thread, null, new Notifier.Line(Notifier.nameOf(context, from), from, body.toString(), now));
    }
}
