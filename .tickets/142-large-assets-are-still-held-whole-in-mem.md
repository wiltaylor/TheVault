---
id: '142'
title: Large assets are still held whole in memory on the server and in the Android
  share path
blocked_by: []
status: done
verified: verified
---

# Large assets are still held whole in memory on the server and in the Android share path

Found while fixing ticket 141. The device now streams asset uploads from disk and downloads to a file (crates/sync/src/client.rs, store Change::Stage), but two paths still hold a whole file in memory: (1) the server's PUT /api/assets/{id} and POST /api/assets take the body as axum Bytes up to MAX_UPLOAD = 512 MB (crates/api/src/assets.rs), so a 200 MB upload costs the server 200 MB of RAM per request; it could stream the body to a file under .vault/incoming and commit it with Writer::put_asset_file_with_id. (2) The Android share path reads each shared file whole with std::fs::read before Writer::put_asset (plugins/tauri-plugin-vault-device/src/share.rs store), where Writer::put_asset_file would move it instead. Also the GET /api/assets/{id} answer path was not checked. Decide whether either matters at the sizes the user shares.

## Worklog

### 2026-10-22T01:46:56Z — wil

Status set to todo.

### 2026-10-22T01:47:29Z — wil

Claimed.

### 2026-10-22T02:36:37Z — wil

Implemented on ticket/140 (c16fdad): PUT/POST /api/assets stream the body to Writer::incoming_file (.vault/incoming) with a MAX_UPLOAD count (413 too_large) and commit with put_asset_file(_with_id); GET /api/assets/{id} whole and ranged now stream from the file (Writer::asset_file, Vault::open_at) with Content-Length; share::store moves each copy with Writer::put_asset_file (the Kotlin ShareImporter already streamed the content:// copy). Tests: crates/server/tests/asset_memory.rs, a 300 MB PUT and GET grow the server's VmHWM by under 64 MB (old code: 997912576 bytes, failed); plugins/.../tests/share_memory.rs, a 300 MB share grows the process by under 64 MB (old code failed). Decision: it matters, fixed for both. Not covered by a test: the 413 path over 512 MB.

### 2026-10-22T02:36:37Z — wil

Verified set to verified.

### 2026-10-22T02:36:37Z — wil

Status set to done.
