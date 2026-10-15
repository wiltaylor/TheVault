# TheVault desktop and Android clients: implementation specification

Source: `MAP.md` in this directory (decisions D1 to D30, D1 struck by D18). Every statement below traces to a decision, a research note, or the code.

## Problem

TheVault is a single-user, self-hosted note app reached through a browser (`crates/server`, `web/`). It has no native client, no offline use, and no way to bring phone data in. The browser cannot be a share target from other Android apps. It cannot read calls, SMS, MMS, RCS or contacts, and it cannot show system notifications when closed. It also loses its session on every server restart, because sessions live in memory (`crates/api/src/auth.rs`).

## Solution

Two Tauri 2 apps from one code base: a Linux desktop app and an Android app (Android 13, API 33, and later) (D14). Windows, macOS and iOS are out of scope.

- **Server inside the app (D18, D26).** Each app runs the server stack in-process (`api`, `writer`, `index`, `store`, `state`, `script`) against a local copy of the vault. It serves the bundled `web/dist` and `/api` through a custom URI scheme with no network port. The web code stays same-origin and unchanged.
- **Offline editing and sync (D8, D19, D20, D21, D27 to D30).** The home server is the master copy. Devices pull a change feed and push queued writes through the normal write API. Conflicts merge three-way by field and by block. A true clash becomes a conflict copy.
- **Devices (D2, D3).** A device pairs once (password or QR), receives a per-device token, and appears in Settings > Devices.
- **Push and live updates (D4, D9).** The server pushes to Android through UnifiedPush (fallback: a WorkManager check every 15 minutes) and streams events to open clients over SSE. Notifications mirror both ways.
- **Device events and scripts (D5, D6, D17, D22, D23).** The Android app captures device events and runs their wscript handlers on the device, writing notes through the local vault. Server scripts can send the device allowlisted commands.
- **Phone data (D7, D13, D15, D16, D24, D25).** Built-in `message`, `call` and `conversation` types; two-way contact sync with `person`; RCS from Google Messages notifications, marked partial.
- **Desktop extras and distribution (D10, D11, D12).** Tray, global hotkey, autostart, `thevault://` links; AppImage, AUR, deb, rpm; self-signed APKs on GitHub Releases.

## User stories

1. As the user, I install the Linux app, enter my server URL and password once, and use TheVault in its own window without signing in again after server restarts.
2. As the user, I pair my phone by scanning a QR code shown in Settings > Devices on the web app.
3. As the user, I see each paired device in Settings > Devices and rename or revoke it; a revoked device can no longer reach the server.
4. As the user, I edit notes on my laptop or phone with no connection, and the edits reach the server when the connection returns.
5. As the user, when I edited the same note on two devices, non-overlapping changes merge, and an overlapping change keeps the server's version and gives me a conflict copy plus a notification.
6. As the user, I share text, a link, an image or a file from any Android app to TheVault and it becomes a fleeting note, even with no connection.
7. As the user, my SMS, MMS and RCS messages and my calls become `message`, `call` and `conversation` notes linked to the right `person`, without my opening the app.
8. As the user, verification codes and anyone on my exclusion list are never captured.
9. As the user, I import past calls and messages for a date range I choose, once.
10. As the user, my phone contacts and my `person` notes stay in step in both directions, and deleting on one side unlinks rather than deletes.
11. As the user, a TheVault notification appears on my phone as an Android notification; swiping it marks it read on the server, and reading it anywhere else clears it from the phone within seconds when a UnifiedPush distributor is installed.
12. As the user, I write wscript handlers in `devices/android.wcl` and `scripts/*.ws` that decide what phone data becomes notes, and edit the default ones.
13. As the user, a server script can ask my phone to send an SMS or update a contact, but only for commands I enabled for that device.
14. As the user, on Linux I capture a note from a global hotkey or the tray, see an unread badge in the tray, get desktop notifications, and open `thevault://` links.
15. As the user, when my app is older than my server, sync stops with a clear "update the app" message and my offline edits wait in the queue.

## Implementation decisions

### Repository layout

- `apps/desktop/` holds the Tauri Linux app; `apps/android/` holds the Tauri Android app, including `src-tauri/gen/android` committed (note 02).
- `crates/script-core/` holds the wscript host engine split out of `crates/script`: `Kind`, `Job`, `Limits`, `Host` and the `Vault` trait, with no dependency on `writer`, `state` or `rusqlite` (D6). `crates/script` keeps the server implementations and re-exports the engine.
- `crates/sync/` holds the device-side sync engine: queue, change-feed client, three-way merge, conflict copies, version handshake (D20, D27 to D29).
- `crates/embedded/` assembles the in-process server for the apps: the existing router with the scheduler and the `after_*` handler queue switched off (D17), plus the sync engine and the device event queue.
- `plugins/tauri-plugin-vault-device/` is our Kotlin Tauri mobile plugin for SMS/MMS, call log, contacts, notification listener, UnifiedPush, WorkManager and JNI entry points (note 01).
- `web/` gains no base-URL changes (D18). It gains the Settings > Devices page, QR pairing, conflict-copy display, and an SSE subscription replacing the 30-second notification poll when available (D4).

### Server: devices and pairing (D2, D3)

- `state.db` gains a `devices` table: `id` (ULID), `name`, `platform` (`linux` | `android`), `capabilities` (JSON), `token_id`, `push_endpoint` (nullable), `last_seen_ms`, `settings` (JSON: enabled commands, enabled write/send switches, exclusion list, skip pattern; D22, D23, D25), `created_ms`. Bump the state schema version with a migration.
- `POST /api/devices/pair` with `{password, name, platform, capabilities}` returns `{device_id, token}`; the token has write scope and is bound to the device row.
- `POST /api/devices/pair-codes` (session or admin) returns a short-lived single-use code and its QR payload `{server_url, code}`; `POST /api/devices/pair` also accepts `{code, ...}` instead of a password.
- `GET /api/devices`, `PATCH /api/devices/{id}` (name, settings), `DELETE /api/devices/{id}` (revokes: deletes the token and the push endpoint).
- `PUT /api/devices/{id}/push` registers or clears the UnifiedPush endpoint and its WebPush keys.
- The CLI mirrors these as `vault device list|rename|revoke|pair-code`.

### Server: change feed and version handshake (D20, D27, D29)

- `GET /api/sync/info` (unauthenticated) returns `{protocol, format_version, server_version}`.
- `GET /api/sync/changes?since=<seq>&limit=` returns, in `versions.seq` order, every change to notes (WCL text and version), types, templates, taxonomies, scripts, tasks and `devices/*.wcl`, and deletions and trash moves, plus `next` and `latest`.
- Writes authenticated by a device token always create a new version (never merged into the head session version) (D29).
- `PUT /api/devices/{id}/bases` records the oldest base version the device still holds per note; history pruning never deletes those versions (D29).
- Types, templates, taxonomies, scripts and tasks are read-only to device sync; devices edit them only while online through the normal API (D20).

### Server: push and live events (D4, D9, D22)

- `GET /api/events` is an SSE stream for an authenticated client: `notification.created`, `notification.read`, `notification.dismissed`, `notification.cleared`, `note.changed`, `device.command`.
- A push dispatcher sends WebPush-encrypted (RFC 8291) messages to each device's UnifiedPush endpoint for notification create, read, dismiss and clear, and for device commands. Messages carry ids only; the device fetches details through the API.
- The web client subscribes to `/api/events` and falls back to its existing 30-second poll if the stream fails.

### Server: device commands (D22)

- `state.db` gains `device_commands`: `id`, `device_id`, `name`, `payload` (JSON), `created_ms`, `expires_ms`, `status` (`pending` | `delivered` | `done` | `failed` | `expired`), `result` (JSON), `source`.
- Scripts call `vault::device_command(device, name, payload)` (in `after_*` handlers and tasks only, like `vault::notify`). The server rejects a command the device's `devices/android.wcl` does not declare or the user did not enable for that device.
- `GET /api/devices/{id}/commands?status=pending` and `POST /api/devices/{id}/commands/{cid}/result` serve the device.

### Vault format: devices and phone types (D5, D15, D24)

- New file kind `devices/<platform>.wcl`. It declares handlers per device event, `handler "<event>" { script, function }`, and the commands the device accepts, `command "<name>" { script, function }`. Document the vocabulary in `docs/vault-format.md` and a structural schema in `docs/vault-format/schema/`.
- Device events (a closed list): `sms_received`, `sms_sent`, `mms_received`, `rcs_received`, `call_ended`, `contact_changed`, `contact_deleted`, `share_received`. Each event's payload shape is fixed in `docs/scripts.md`.
- Built-in commands the device module implements: `send_sms`, `upsert_contact`, `delete_contact`, `show_notification`.
- New built-in types (like `dailynote` and `fleeting`: customisable, not deletable), added through the existing built-in type list:
  - `message`: `channel` (select: sms, mms, rcs), `direction` (select: in, out), `from` and `to` (reference to `person`, multiple for `to`), `from_raw` and `to_raw` (strings: number or display name), `conversation` (reference), `sent` (datetime), `body` (content), `attachments` (asset, multiple), `partial` (bool), `device_ref` (string: provider row id or notification key, for de-duplication).
  - `call`: `direction` (select: in, out, missed), `person` (reference), `number` (string), `started` (datetime), `duration` (int, seconds), `device_ref` (string).
  - `conversation`: `participants` (reference to `person`, multiple), `channel`, `thread_ref` (string); its default template shows a query view of its messages.
- `person` (starter type) gains `phones` (list of strings, E.164), `addresses` (list of strings) and a hidden `device_contacts` dict (device id to contact id) (D24).
- New canvas group, node and edge ids get a short random suffix (D30).
- Default device scripts ship in the starter vault and as built-in defaults: `scripts/device_messages.ws`, `scripts/device_calls.ws`, `scripts/device_contacts.ws`, `scripts/device_share.ws`, wired by a default `devices/android.wcl`.

### Script engine on the device (D6, D17, D23, D25)

- `crates/script-core` runs on Android through the Rust core. The device's `Vault` implementation writes to the local vault through `crates/embedded`; those writes sync like any other.
- A `device` module, available only to device-run scripts: `sms.list`, `sms.get`, `calls.list`, `contacts.list`, `contacts.get` (reads, available when Android granted the permission); `contacts.upsert`, `contacts.delete` (behind the device's contacts-write switch); `sms.send` (behind the SMS-send switch and SEND_SMS).
- Before a handler runs, the engine drops events matching the device's exclusion list (contact, number or thread) and SMS matching the skip pattern (default: one-time codes and verification texts) (D25).
- Number matching normalises to E.164 with the phone's region (libphonenumber on the Kotlin side passes normalised numbers in the payload); RCS senders match `person` by exact name or the phone contact's display name. Unknown numbers create no `person` (D24).

### Android app (D7, D9, D10, D13, D14, D16, D17)

- Kotlin plugin `tauri-plugin-vault-device`:
  - Receivers for SMS received, MMS (by observing the Telephony provider, not parsing WAP push), and phone state for call end, which then re-reads the call log.
  - A `NotificationListenerService` that parses Google Messages notifications into `rcs_received` events with `partial = true` (D13).
  - A `ContentObserver` on contacts and a WorkManager content-URI trigger job for changes while the app is not running.
  - A periodic WorkManager catch-up that reads the providers since a stored cursor; events de-duplicate on `device_ref` (D7).
  - A first-run import screen: off by default, the user picks a date range (D7).
  - JNI entry points into the Rust core to enqueue events and run handlers with no webview (D17).
- Share target through `tauri-plugin-mobile-sharetarget` (or a fork). Shared `content://` files are copied natively into assets of the local vault (note 01).
- Notifications: the official `tauri-plugin-notification` shows and cancels notifications. Android notification ids derive from server notification ids. A swipe marks the notification read through the API; a read, dismiss or clear push cancels it (D9).
- UnifiedPush through `org.unifiedpush.android:connector`; the endpoint registers with `PUT /api/devices/{id}/push`. With no distributor, a WorkManager check every 15 minutes (D4).
- Onboarding: server URL plus password or QR scan, then each permission in turn, including the Allow restricted settings steps for SMS and notification-listener access (note 02).
- Device token in the Android Keystore (D2). Minimum SDK 33.
- Release: APK signed with a permanent keystore kept outside the repo, published on GitHub Releases for Obtainium or `adb` (D10).

### Linux desktop app (D11, D12, D21, D27)

- Tauri app with the in-process server (`crates/embedded`), full offline (D21), and every asset kept locally (D19).
- Tray icon with unread badge and Quick capture; desktop notifications through the notification daemon, mirrored and cleared like Android; start on login minimised; global capture hotkey through the XDG GlobalShortcuts portal, falling back to `thevault --capture` for a user-bound key; `thevault://note/<id>` links.
- Device token in the Secret Service keyring (D2).
- Update check against GitHub Releases (D27). Release artefacts: AppImage, deb, rpm, and an AUR `thevault-bin` PKGBUILD.

### Sync engine (D19, D20, D27 to D29)

- The device stores every note (text) and, on Android, assets on demand with a size-limited cache that prefetches assets of pinned and recently opened notes. The desktop keeps all assets (D19).
- Local writes go to the local vault and into a durable queue in the local `state.db`, each with its base version and the base WCL text (D29).
- Push: replay queued writes in order through the normal write API with `If-Match`. On 409, merge base, device and server: fields individually; content blocks by a diff3 over hashes of each top-level block's canonical text; ambiguous alignment or overlapping edits keep the server version and create a conflict copy titled `<title> (conflict, <device name>)`, linked to the original, plus a notification (D20, D28).
- Pull: `GET /api/sync/changes` since the stored cursor; apply to the local vault and reindex.
- Handshake: call `/api/sync/info` first; refuse to sync and keep the queue when the server's protocol or format version is newer than the app supports (D27).
- The local server never runs the scheduler or `after_*` handlers; validation and `before_*` handlers run locally on write and again on the server when the write arrives (D17).

## Testing decisions

The seams were confirmed by the user before writing.

- **S1, Rust server integration (`crates/testkit`, existing):** pairing by password and by code, token scope and revocation, the devices API, `/api/sync/info`, `/api/sync/changes` ordering and coverage of every file kind and deletion, device writes never merging into a session version, base-version retention through pruning, the SSE stream event set, WebPush dispatch to a fake push endpoint on port 0 (payload decrypts, ids only), device command allowlisting, expiry and results, `vault::device_command` availability by script kind.
- **S2, Playwright (`web/e2e`, existing):** Settings > Devices list, rename, revoke, switches, exclusion list and skip pattern; QR pairing code display; conflict copies shown with a link to the original; live notification updates over SSE with fallback to polling.
- **S3, two-node sync harness (new, Rust, in `crates/sync` tests):** a device node built from `crates/embedded` and a testkit home server in one process. Cases: offline edits on both sides to different fields and different blocks merge; overlapping edits produce one conflict copy and one notification; reordered blocks fall back to a conflict copy; a newer server refuses sync and keeps the queue; the base survives a browser session merge and pruning on the server; deletions and trash moves propagate; definitions are not editable through device sync.
- **S4, device script tests (new, Rust, in `crates/script-core` or `crates/embedded` tests):** run the default device scripts against a fake `device` module and canned events. Cases: SMS, MMS, RCS and calls become `message`, `call` and `conversation` notes with correct `person` links; E.164 and name matching; no `person` for unknown numbers; exclusion list and skip pattern; de-duplication on `device_ref`; contact sync both ways with unlink-on-delete; write and send switches enforced; command allowlist enforced.
- **S5, Kotlin unit tests with Robolectric (new, JVM in the dev container):** provider readers for SMS, MMS, call log and contacts; receivers enqueue the right events; Google Messages notification parsing into `rcs_received`; UnifiedPush message handling cancels and shows notifications by id.
- **S6, desktop end-to-end with `tauri-driver` (new):** under Xvfb with WebKitGTK, the app starts, pairs with a testkit server, edits offline, syncs after the server comes back, and updates the tray badge from a server notification.
- **S7, manual phone checklist (new, run by the user on a real phone):** share target from three apps, a live SMS, an MMS with an image, an RCS message, a call, contact edits in both directions, onboarding including Allow restricted settings, a UnifiedPush notification shown and cleared from the web app within seconds, offline capture synced later.

## Out of scope

- Windows, macOS and iOS clients.
- Google Play distribution and FCM push (note 02, D10).
- Editing types, templates, taxonomies, scripts or tasks offline (D20).
- Running scheduled tasks or `after_*` handlers on a device (D17).
- Being the default SMS or dialer app; placing calls.
- Complete RCS capture (D13: messages read in an open chat, sent RCS and attachments are not captured).
- Flatpak (D11).

## Open questions

- The exact payload of each device event and command is listed but not field-by-field specified here; the first device ticket writes it into `docs/scripts.md` and S4 tests pin it. (Accepted unknown from the map: de-duplication keys, D7.)
- The server crates are expected to build for Android aarch64 (tantivy, bundled rusqlite, pdf-extract); the first Android ticket confirms it before the rest of the Android work starts. (Accepted unknown, D18.)
- Range requests and streaming uploads through the Tauri custom scheme are expected to work; the embedded-server ticket confirms it with pdf.js. (Accepted unknown, D26.)
- How a conflict copy links to its original (a reference field or a mention in its body) is left to the sync ticket. (Accepted unknown, D20.)
- RCS capture depends on Google Messages' notification layout, which can change without notice (D13); S5 pins the parser to recorded samples, and S7 is the only check against the live app.
