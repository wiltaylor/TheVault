---
id: '142'
title: Large assets are still held whole in memory on the server and in the Android
  share path
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# Large assets are still held whole in memory on the server and in the Android share path

Found while fixing ticket 141. The device now streams asset uploads from disk and downloads to a file (crates/sync/src/client.rs, store Change::Stage), but two paths still hold a whole file in memory: (1) the server's PUT /api/assets/{id} and POST /api/assets take the body as axum Bytes up to MAX_UPLOAD = 512 MB (crates/api/src/assets.rs), so a 200 MB upload costs the server 200 MB of RAM per request; it could stream the body to a file under .vault/incoming and commit it with Writer::put_asset_file_with_id. (2) The Android share path reads each shared file whole with std::fs::read before Writer::put_asset (plugins/tauri-plugin-vault-device/src/share.rs store), where Writer::put_asset_file would move it instead. Also the GET /api/assets/{id} answer path was not checked. Decide whether either matters at the sizes the user shares.

## Worklog
