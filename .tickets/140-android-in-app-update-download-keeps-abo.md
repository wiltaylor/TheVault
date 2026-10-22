---
id: '140'
title: Android in-app update download keeps aborting (client resets the connection
  mid-download), so a phone gated on a newer server never updates
blocked_by: []
status: claimed
verified: unknown
---

# Android in-app update download keeps aborting (client resets the connection mid-download), so a phone gated on a newer server never updates

Seen 2026-10-21 on severus (vault.wil.dev, 0.1.12) behind Caddy on the LAN: the phone's old app (Dalvik/2.1.0, Android 17, SM-F976B) downloaded /api/clients/TheVault_0.1.12_aarch64.apk four times between 06:22 and 06:23 UTC; Caddy logged each as 'aborting with incomplete response ... writing: write tcp 172.29.0.2:443->10.3.0.144: write: connection reset by peer', so the phone closed the connection. The version gate (ticket 112) kept sync paused meanwhile, showing 'changes waiting'. The same file then downloaded fine in the phone's browser (Chrome, one http2 stream close, resumed with Range bytes=23251414-). Suspects: AppUpdateWorker's 60 s readTimeout or HttpURLConnection behaviour on a 68 MB body, WorkManager stopping the job (constraints or the 10-minute limit), or Android 17 background network limits. Reproduce with adb logcat on a phone against a server serving a newer APK; consider DownloadManager (system download service, resumable, survives process death) instead of HttpURLConnection in a Worker.

## Worklog

### 2026-10-22T01:46:56Z — wil

Status set to todo.

### 2026-10-22T01:47:29Z — wil

Claimed.
