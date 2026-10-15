package com.google.android.apps.messaging;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

/** TEST ONLY. Needed for the SMS role; the emulator delivers no MMS, and CommandReceiver inserts them. */
public class MmsDeliverReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context context, Intent intent) {}
}
