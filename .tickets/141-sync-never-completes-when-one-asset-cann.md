---
id: '141'
title: 'Sync never completes when one asset cannot upload: flat 30 s timeout, whole-file
  uploads, and a failed asset blocks every note; the popover shows no error'
blocked_by: []
status: done
verified: verified
---

# Sync never completes when one asset cannot upload: flat 30 s timeout, whole-file uploads, and a failed asset blocks every note; the popover shows no error

Live bug, 2026-10-21: a phone (SM-F976B, Android 17, app 0.1.12, clean install) paired with https://vault.wil.dev (severus, 0.1.12) never completes a sync round. After "Import from phone" it shows "changes waiting" forever; the sync popover says "Last sync: Never" and shows no error. Server side: 18 assets arrived (PUT /api/assets/{id}) at 06:44, then nothing; no note was ever pushed.

Cause found by code reading (not yet confirmed on the phone): a round uploads every local asset the server lacks before pushing any note (push_assets, crates/sync/src/lib.rs ~752); an asset is marked uploaded only after success; the sync HTTP client sets no timeouts (crates/sync/src/client.rs ~43-45), so reqwest's blocking default of 30 s applies to the whole request including the upload body; the whole asset is read into memory and sent as one body (lib.rs ~761, client.rs ~124-130). One asset that cannot upload within 30 s (a large MMS part, or a slow or stalling network path: the same phone also failed to download the 68 MB APK repeatedly from the same server, see ticket 140) fails the round at lib.rs ~316 before notes are pushed, and every later round fails on the same asset. MMS import stores every non-text part as an asset with no size cap and reads it whole into memory (MmsReader.kt ~112-139).

Fix:
1. HTTP timeouts in the sync client: an explicit connect timeout (for example 15 s), no flat total timeout on uploads and downloads; instead a stall timeout (no bytes moved for, say, 60 s) so a slow but progressing transfer completes and a dead one fails. Keep sensible timeouts on small JSON requests.
2. Stream asset uploads from the file (and asset downloads to a temp file then rename) instead of whole-body in memory.
3. An asset that fails to upload does not fail the round: record the failure (asset id, filename, size, error, attempts), skip it, push the notes (a note naming a missing asset may be refused by the server's validation: check whether it is, and if so push the notes that do not depend on failed assets and keep the rest queued with the reason), retry the failed asset on later rounds with backoff.
4. Status: the sync popover shows the last error whenever a round failed or partly failed (not only when red), the number of assets and notes still waiting, and names a stuck asset (filename, size, last error). "Last sync" shows the last successful round, or "Never" plus the last attempt time and its error.
5. MMS capture: cap what is stored as an asset at a size (for example 50 MB) and stream the part to disk rather than into memory; parts over the cap are listed on the message note by name and size, like oversized Gmail attachments.
6. Logging: each round logs one line at info with its outcome (pushed, pulled, assets sent, failures) under the TheVault tag, so adb logcat shows progress.

Acceptance criteria:
- Tests in crates/sync against a test server: a slow asset upload that keeps moving (throttled to finish after 45 s) completes; a stalled upload fails after the stall timeout without failing the round; with one asset failing, the notes that do not depend on it are pushed and the status reports the failed asset; a later round uploads it once the server accepts it.
- Streaming: a 200 MB asset uploads without the process holding it in memory (assert peak memory or that the body is a stream).
- Status JSON and the web popover show the last error and the stuck asset (web component test).
- MMS cap test in Robolectric.
- ./dev just check and ./dev just android-test pass.

## Worklog

### 2026-10-21T06:57:46Z — wil

Claimed.

### 2026-10-21T07:28:39Z — wil

Done on ticket/141 (fade24d). Proven: crates/sync/tests/asset_transfers.rs against a testkit server behind a TCP proxy in the test: a 9 MiB upload throttled to 200 KiB/s takes about 46 s with the default timeouts and completes (the old flat 30 s timeout would have failed it); with every connection stalled after 1 MiB a 32 MiB upload fails after the 2 s stall timeout set in the test, the round still returns Ok, the two notes that do not name it are pushed, the one that does is held, Report.stuck and GET /api/app/sync name big.mp4 with size, error and attempts; the next round leaves it alone during its retry wait, and after retry_assets_now the next round uploads it and pushes the held note; an asset the server refuses with 409 does not stop the round. Streaming: client::tests uploads a 200 MiB file to a counting axum server with peak RSS growth under 64 MiB, and the body is a stream (as_bytes is None). Status: embedded sync_status unit test and home.rs JSON; web SyncStatus.test.tsx renders the popover details for the phone's case (Never (last tried 2 min ago), stuck VID_0042.mp4 (72.0 MB) with its error, Last error while yellow). MMS: MmsCapTest (Robolectric, 3 tests) proves parts over the cap are listed in the body and not stored, the cap is 50 MB, and the production store streams a part through a file that VaultCore.putAssetFile moves into the vault, reading at most 64 KiB at once. Server check: notes create/update validate asset fields (schema::note FieldKind::Asset: asset X does not exist), so notes naming a failed asset are held, not pushed. ./dev just check exit 0; ./dev just android-test exit 0. Not tried on the phone itself. Timeouts: connect 15 s, JSON request 60 s, stall 60 s (plus up to 512 s grace after the last block for OS buffers to drain), retry 30 s doubling to 1 h. Filed 142 for the server's and the share path's whole-file buffers. The desktop Cargo.lock was at 0.1.11 on main; cargo restamped it to 0.1.12.

### 2026-10-21T07:28:39Z — wil

Verified set to verified.

### 2026-10-21T07:28:39Z — wil

Status set to done.
