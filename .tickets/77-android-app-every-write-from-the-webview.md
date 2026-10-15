---
id: '77'
title: 'Android app: every write from the webview fails, because the custom-scheme
  request reaches the embedded server with an empty body'
plan: thevault-clients
blocked_by: []
status: todo
verified: unknown
---

# Android app: every write from the webview fails, because the custom-scheme request reaches the embedded server with an empty body

Found by the emulator run of docs/android-manual-checks.md (check 8; Android 15 API 35 x86_64 emulator, debug APK at 1574931 with the ticket 76 lock workaround applied so the app starts).

Repro: pair the app, open any note, change its title (or create a note, or a block). Or, from the app's WebView: fetch('/api/notes?type=note', {method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify({note: {fields: {title: 'probe'}, blocks: []}})}).

Expected: the write is saved in the local vault and syncs to the server (check 8: the edit appears in the web app within seconds).

Actual: 400 {"error":"bad_request","message":"the body is not JSON: EOF while parsing a value at line 1 column 0"}. The editor shows that message in red under the note toolbar, and nothing is saved or synced. GETs work, so notes show; no POST, PUT or PATCH from the web client works, so the app is read-only apart from shares and device capture (which go through Kotlin/JNI, not the webview).

Cause: apps/android/src-tauri/src/lib.rs serves /api through register_asynchronous_uri_scheme_protocol. On Android, wry implements custom schemes with WebViewClient.shouldInterceptRequest, whose WebResourceRequest carries no request body, so every request arrives with an empty body. The desktop (WebKitGTK) path does carry bodies, which is why S6 passes.

Fix direction: route webview API writes another way on Android, e.g. a fetch shim in the web client for the app that sends bodies through a Tauri command / IPC (which has its own body channel), or serve the embedded API on a loopback port with a per-launch token. Add an on-device (instrumented or emulator) test of a POST through the scheme.

Evidence: .scratch/emulator/evidence/webview-write-empty-body.png; probe output recorded in .scratch/emulator/RESULTS.md (check 8).

## Worklog
