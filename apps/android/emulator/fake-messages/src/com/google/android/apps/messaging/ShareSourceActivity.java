package com.google.android.apps.messaging;

import android.app.Activity;
import android.content.ClipData;
import android.content.ComponentName;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.pdf.PdfDocument;
import android.net.Uri;
import android.os.Bundle;
import android.util.Log;
import java.io.File;
import java.io.FileOutputStream;
import java.util.ArrayList;

/**
 * TEST ONLY. Shares to a target as another app would. Extras: target (a component, default
 * TheVault's debug ShareActivity), files (comma-separated names ending .png or .pdf, made here),
 * text, subject. One file or none is ACTION_SEND; more is ACTION_SEND_MULTIPLE.
 */
public class ShareSourceActivity extends Activity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        try {
            share(getIntent());
        } catch (Exception e) {
            Log.e(Notifier.TAG, "share failed", e);
        }
        finish();
    }

    private void share(Intent in) throws Exception {
        String target = in.getStringExtra("target");
        if (target == null) target = "dev.wiltaylor.thevault.debug/dev.wiltaylor.thevault.device.ShareActivity";
        ArrayList<Uri> uris = new ArrayList<>();
        String files = in.getStringExtra("files");
        if (files != null && !files.isEmpty()) {
            for (String name : files.split(",")) uris.add(make(name.trim()));
        }
        Intent out = new Intent(uris.size() > 1 ? Intent.ACTION_SEND_MULTIPLE : Intent.ACTION_SEND);
        out.setComponent(ComponentName.unflattenFromString(target));
        String type = "text/plain";
        if (!uris.isEmpty()) {
            boolean allImages = true;
            for (Uri u : uris) allImages &= u.getLastPathSegment().endsWith(".png");
            type = uris.size() == 1 ? getContentResolver().getType(uris.get(0)) : (allImages ? "image/*" : "*/*");
        }
        out.setType(type);
        if (in.hasExtra("text")) out.putExtra(Intent.EXTRA_TEXT, in.getStringExtra("text"));
        if (in.hasExtra("subject")) out.putExtra(Intent.EXTRA_SUBJECT, in.getStringExtra("subject"));
        if (uris.size() == 1) out.putExtra(Intent.EXTRA_STREAM, uris.get(0));
        if (uris.size() > 1) out.putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris);
        if (!uris.isEmpty()) {
            ClipData clip = ClipData.newRawUri("shared", uris.get(0));
            for (int i = 1; i < uris.size(); i++) clip.addItem(new ClipData.Item(uris.get(i)));
            out.setClipData(clip);
            out.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        }
        startActivity(out);
        Log.i(Notifier.TAG, "shared " + uris + " to " + target);
    }

    private Uri make(String name) throws Exception {
        File f = new File(SharedFiles.dir(this), name);
        try (FileOutputStream out = new FileOutputStream(f)) {
            if (name.endsWith(".pdf")) {
                PdfDocument pdf = new PdfDocument();
                PdfDocument.Page page = pdf.startPage(new PdfDocument.PageInfo.Builder(200, 200, 1).create());
                Paint p = new Paint();
                p.setTextSize(16);
                page.getCanvas().drawText("TheVault test PDF " + name, 10, 100, p);
                pdf.finishPage(page);
                pdf.writeTo(out);
                pdf.close();
            } else {
                Bitmap b = Bitmap.createBitmap(48, 48, Bitmap.Config.ARGB_8888);
                Canvas c = new Canvas(b);
                c.drawColor(Color.rgb(Math.abs(name.hashCode()) % 255, 120, 60));
                b.compress(Bitmap.CompressFormat.PNG, 100, out);
            }
        }
        return Uri.parse("content://" + SharedFiles.AUTHORITY + "/" + name);
    }
}
