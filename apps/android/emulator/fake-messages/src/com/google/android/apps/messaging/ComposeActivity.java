package com.google.android.apps.messaging;

import android.app.Activity;
import android.os.Bundle;

/** TEST ONLY. Needed for the SMS role. */
public class ComposeActivity extends Activity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        finish();
    }
}
