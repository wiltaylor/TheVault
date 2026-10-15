package com.google.android.apps.messaging;

import android.app.Service;
import android.content.Intent;
import android.os.IBinder;

/** TEST ONLY. Needed for the SMS role. */
public class RespondService extends Service {
    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        stopSelf(startId);
        return START_NOT_STICKY;
    }
}
