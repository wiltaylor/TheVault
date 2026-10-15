---
slug: thevault-clients
status: specified
created: 2026-10-15
---

# TheVault Linux desktop and Android clients

## Destination
TheVault ships a Linux desktop app built with Tauri and an Android app built with Tauri mobile. Both reuse the existing SolidJS web front end and connect to the user's self-hosted server. Windows, macOS and iOS are out of scope. The Android app is a share target that turns shared text, links, images and files into fleeting notes. It reads call history and SMS/MMS (and RCS where the platform exposes it), reads and writes the phone's contacts, and shows TheVault notifications as Android notifications that clear when cleared elsewhere and the other way round. Device data is read and written through events handled by wscript: scripts run on the server by default, and a defined set of device events (an SMS arriving, a call ending, a contact changing) run on the device. The deliverable of this map is a specification for building both apps; nothing is built during mapping.

## Grounding
- The web client calls only relative `/api/...` paths through one helper with `credentials: "same-origin"`; there is no base-URL setting (`web/src/api.ts:353-388`, endpoints `api.ts:393-551`).
- `assetUrl()` returns relative `/api/assets/<id>`, used as plain `<img src>`, links, a pdf.js URL and an OpenLayers URL, so asset loads rely on the cookie (`web/src/api.ts:570-572`, `web/src/blocks/MediaBlocks.tsx:135`, `web/src/pdf/PdfPreview.tsx:119`, `web/src/map/MapCanvas.tsx:56`).
- Routing and link handling assume the page origin is the server origin (`web/src/App.tsx:21-41`, `web/src/workspace/events.ts:29-30`, `web/src/workspace/model.ts:410-412`).
- Browser auth is a `tv_session` cookie, `HttpOnly; SameSite=Strict`, 30-day TTL, held in memory so a server restart signs everyone out (`crates/api/src/auth.rs:26-33`, `auth.rs:98-130`, `auth.rs:236-242`).
- Bearer tokens with read/write/admin scopes already work and win over the cookie; tokens are the only per-device identity today (`crates/api/src/auth.rs:159-187`, `crates/state/src/auth.rs:14-45`, `crates/state/src/lib.rs:136-142`).
- The server has no CORS, no TLS and no push channel (no SSE or websocket anywhere); `--listen` defaults to `127.0.0.1:8080` and prints `{"listen": addr}` on start (`crates/server/src/main.rs:22-24`, `main.rs:136-143`, `crates/server/src/web.rs:18-91`).
- The PWA has a manifest with no `share_target` and SVG icons only, and a service worker that never caches `/api` (`web/public/manifest.webmanifest:1-14`, `web/public/sw.js:19-73`).
- Quick capture uploads each image with `POST /api/assets` and creates the note with `POST /api/notes?type=fleeting`; no route opens capture with pre-filled text (`web/src/capture/CaptureModal.tsx:75-94`, `web/src/routes.tsx:60`).
- Notifications live in `state.db` (`notifications` table) with read, read-all, dismiss and clear endpoints; the web client polls every 30 s and on focus (`crates/state/src/lib.rs:229-239`, `crates/api/src/lib.rs:159-163`, `web/src/notifications/store.ts:19-93`).
- Handler events are a closed list of six note events (`before_`/`after_` create, update, delete); no custom or device events and no inbound event endpoint exist (`crates/schema/src/types.rs:17-24`, `types.rs:810-813`, `docs/parity.md:155-240`).
- `after_*` handlers queue in `handler_jobs` and run on a server thread; host modules per kind are fixed (`crates/script/src/handlers.rs:13-45`, `crates/script/src/lib.rs:14-27`, `lib.rs:89-117`, `docs/scripts.md:5-15`).
- Notes carry opaque versions with `ETag`/`If-Match` conflict checks and a 409 on a stale base; there is no change feed or `since` endpoint (`crates/api/src/notes.rs:35-80`, `crates/writer/src/lib.rs:90-100`, `crates/state/src/lib.rs:108-124`).
- `POST /api/upsert` and `/api/batch` upsert by a key field with dry run, the closest thing to an ingest path (`crates/api/src/batch.rs:1-14`).
- The starter `person` type has email, organisation, homepage and body only: no phone numbers, addresses or external id; no vCard handling exists (`crates/cli/starter/types/person.wcl:1-65`).
- wscript's core, compiler and VM have no third-party dependencies and no `unsafe`, so they should build for Android; `crates/script` itself depends on server-only crates (writer, state, rusqlite), but host functions go through a `trait Vault: Send + Sync` seam (`crates/script/Cargo.toml`, `crates/script/src/vault.rs:16`, `vault.rs:264`, `handlers.rs:589`).
- The original PRD made native clients and offline use non-goals (D18); prior art rejected Electron (`docs/plans/thevault-prd/PRD.md:185`, `PRD.md:223-224`, `docs/plans/thevault-prd/notes/21-clients.md`, `notes/02-prior-art.md:41`).
- `web/dist` is 12 MB, main chunk about 2.2 MB; mermaid, OpenLayers, pdf.js and lucide load lazily (`web/src/blocks/MermaidBlock.tsx:16`, `web/src/map/MapCanvas.tsx:13`, `web/src/pdf/load.ts:9`).

## Decisions
| ID | Decision | Struck by | Recorded in |
|----|----------|-----------|-------------|
| D1 | ~~Webview loads the server URL; bundled native page for setup and unreachable state~~ — D18 runs the server stack in the app instead | D18 | `notes/03-front-end-delivery.md` |
| D2 | Native setup mints a per-device token (keystore/keyring), exchanged for a webview session; QR pairing as an alternative | | `notes/04-sign-in.md` |
| D3 | `devices` table in state.db and a Settings > Devices page with rename and revoke | | `notes/05-device-registry.md` |
| D4 | UnifiedPush with a 15-minute WorkManager fallback; server SSE stream for open clients | | `notes/06-push.md` |
| D5 | Fixed device event catalogue, handlers in `devices/android.wcl`, device events run on the device, scripts synced from the vault | | `notes/07-device-events.md` |
| D6 | wscript embedded on Android; engine split from server-only crate; `vault` over HTTP/replica plus a `device` module | | `notes/08-device-runtime.md` |
| D7 | Live capture plus periodic catch-up from a cursor, optional first-run import off by default | | `notes/09-capture-mode.md` |
| D8 | Full offline editing with sync (user's choice over queue-only) | | `notes/10-offline.md` |
| D9 | Swipe marks read on the server; read elsewhere cancels on the phone via push | | `notes/11-notification-mirroring.md` |
| D10 | Self-signed APKs on GitHub Releases via adb or Obtainium; keystore backed up; no Play | | `notes/12-distribution.md` |
| D11 | AppImage, AUR `thevault-bin`, deb and rpm; no Flatpak | | `notes/13-linux-packaging.md` |
| D12 | Tray with badge and capture, mirrored notifications, autostart, global hotkey via portal, `thevault://` links | | `notes/14-desktop-features.md` |
| D13 | RCS captured from Google Messages notifications, marked lossy | | `notes/15-rcs.md` |
| D14 | Minimum Android 13 (API 33) | | `notes/16-min-android.md` |
| D15 | Built-in `message`, `call` and `conversation` types filled by editable default device scripts; one note per message and per call | | `notes/17-phone-data-model.md` |
| D16 | Two-way contact sync with `person` notes; per-field most-recent wins; deletes unlink unless opted in | | `notes/18-contacts-sync.md` |
| D17 | Background device work via JNI into the Rust core; durable local queue; tasks and after-handlers run only on the home server | | `notes/19-background-execution.md` |
| D18 | Apps run the server stack in-process on a local vault copy, serving bundled `web/dist` and `/api`, syncing with the home server | | `notes/20-embedded-server.md` |
| D19 | All notes on every device; Android assets on demand with a size-limited cache; desktop keeps all assets | | `notes/21-offline-scope.md` |
| D20 | Home server is master; change feed from `versions.seq`; three-way field/block merge; clashes become conflict copies; definitions online-only | | `notes/22-sync-conflicts.md` |
| D21 | Desktop works offline too | | `notes/23-desktop-offline.md` |
| D22 | Server-to-device commands via push, limited to declared and enabled commands | | `notes/24-device-commands.md` |
| D23 | `device` module: read SMS, calls, contacts; contact writes and SMS sending behind per-device switches | | `notes/25-device-module.md` |
| D24 | `person` gains phones (E.164) and addresses; match by number, RCS by name; no auto-created people; cross-stored ids | | `notes/27-matching-people.md` |
| D25 | Keep everything; per-device exclusion list and a default skip pattern for one-time codes; vault deletes never touch the phone store | | `notes/28-privacy-retention.md` |
| D26 | Webview reaches the in-process server through a custom URI scheme, no network port | | `notes/29-local-transport.md` |
| D27 | Sync checks protocol and format versions; refuses and queues when the server is newer; desktop update check, Obtainium on Android | | `notes/30-version-skew.md` |
| D28 | Positional diff3 merge over block hashes, fields merged individually, ambiguous cases become conflict copies | | `notes/31-merge-strategy.md` |
| D29 | Device keeps base texts; device writes always add a version; pruning keeps bases devices depend on | | `notes/32-merge-base.md` |
| D30 | New canvas ids get a short random suffix | | `notes/33-canvas-ids.md` |

## Frontier
Session boundary offered 2026-10-15 (cut after foundations, before the phone data model); declined, mapping in one session.
- [x] Q1 (decision) — Front-end delivery: does the app's webview load the server URL, or bundle `web/dist` and call the server cross-origin? (settled: D1)
- [x] Q2 (decision) — App sign-in: how does an app get credentials (password login that mints a per-device token, QR pairing from the web app, or the cookie)? (settled: D2)
- [x] Q3 (decision) — Device registry: does the server record each app install as a device (name, platform, capabilities, token, last seen)? (settled: D3)
- [x] Q4 (decision) — Push channel: how do server events (notifications, device commands) reach a phone whose app is closed? (settled: D4)
- [x] Q5 (research) [status: answered] — Tauri 2 on Android: what plugins or custom Kotlin code are needed for the share intent, Telephony SMS/MMS provider, CallLog provider, ContactsContract read/write, broadcast receivers for SMS and calls, background work and notifications, and what is RCS access in practice?
- [x] Q6 (research) [status: answered] — Google Play policy for READ_SMS, RECEIVE_SMS, READ_CALL_LOG and WRITE_CONTACTS, and what sideload, F-Droid or Obtainium distribution implies.
- [x] Q7 (decision) — Device event model: what event names exist (for example `sms_received`, `call_ended`, `contact_changed`), where handlers for them are declared, and which run on the device versus the server? (settled: D5)
- [x] Q8 (decision) — Device script runtime: does the Android app embed wscript with a `device` module and a `vault` module backed by the HTTP API, or does the device only forward raw events to the server? (settled: D6)
- [x] Q9 (decision) — Capture mode: live as events happen, periodic catch-up from the providers since a cursor, or both, and is there a first-run backfill? (settled: D7)
- [x] Q10 (decision) — Offline: does the app queue captures and device events when the server is unreachable, and is anything else available offline? (settled: D8)
- [x] Q11 (decision) — Notification mirroring: what does dismissing an Android notification do on the server (mark read or dismiss), and how fast must a read elsewhere clear it on the phone? (settled: D9)
- [x] Q12 (decision) [blocked_by: ~~Q7 (D5)~~] — Vault data model for calls and messages: built-in types (call, message, conversation), one note per message or per thread per day, and how MMS attachments are stored. (settled: D15)
- [x] Q13 (decision) [blocked_by: ~~Q7 (D5)~~] — Contacts sync: one-way phone to vault, vault to phone, or two-way, and what wins on conflict? (settled: D16)
- [x] Q14 (decision) [blocked_by: ~~Q12 (D15)~~, ~~Q13 (D16)~~] — Matching people: how a phone number or address maps to a `person` note, and the fields `person` gains (phones, addresses, device contact id). (settled: D24)
- [x] Q15 (decision) [blocked_by: ~~Q6 (notes/02)~~] — Android distribution and signing. (settled: D10)
- [x] Q16 (decision) — Linux desktop packaging: AppImage, deb, rpm, Flatpak, AUR. (settled: D11)
- [x] Q17 (decision) — Desktop-only features: tray icon, global capture hotkey, autostart, notifications through the desktop's notification daemon. (settled: D12)
- [x] Q18 (decision) — RCS: ship a lossy RCS capture mode that reads Google Messages notifications through a NotificationListenerService, or leave RCS out? (settled: D13)
- [x] Q19 (decision) — Minimum Android version for the app (Tauri's floor is Android 7.0, API 24). (settled: D14)
- [x] Q20 (decision) [blocked_by: ~~Q8 (D6)~~] — Background execution: device scripts triggered while no window is open run through JNI into the Rust core from Kotlin receivers and WorkManager workers; is that the model, and where does the on-device event queue live? (settled: D17)

- [x] Q21 (decision) — Front-end delivery under offline editing (D8 reopens D1): keep the remote-loaded UI with a service worker caching the app shell, or bundle `web/dist` and have the native side serve `/api` from a local replica that syncs with the server? (settled: D18)
- [x] Q22 (decision) — Offline data scope: a full replica of notes and assets on each device, notes only with assets on demand, or a chosen subset (pinned, recent, selected types)? (settled: D19)
- [x] Q23 (decision) — Sync and conflicts: how changes flow (a server change feed from `versions.seq`, device push of queued writes) and what happens when the same note changed on two devices (field-level merge, block-level merge, conflict copy)? (settled: D20)
- [x] Q24 (decision) — Does the Linux desktop app work offline too, or only Android? (settled: D21)
- [x] Q26 (decision) — Server-to-device commands: can a server script ask a device to act (send an SMS, write a contact, show a notification), and how is that delivered and authorised? (settled: D22)
- [x] Q27 (decision) — The `device` module: which functions exist (read SMS, read call log, read and write contacts, send SMS) and which need an explicit opt-in per device. (settled: D23)

- [x] Q28 (decision) — Privacy and retention for phone data: what is kept, for how long, and can contacts or threads be excluded from capture? (settled: D25)
- [x] Q30 (decision) — Transport between the app's webview and its in-process server: a custom URI scheme handled in-process, or a localhost port protected by a per-launch secret? (settled: D26)
- [x] Q31 (decision) — Version skew: the app ships its own UI and server code (D18); what happens when the app and home server versions differ during sync? (settled: D27)
- [x] Q32 (research) [status: answered] — Do content blocks have a stable identity today that a block-level three-way merge can key on (D20), and what would adding one cost?

- [x] Q33 (decision) — Merge strategy given no block ids (note 26): positional diff3 over block hashes, add block ids to the format, or field-level clashes only? (settled: D28)
- [x] Q34 (decision) — Merge base preservation: browser saves merge into the head version in place and pruning can delete versions (note 26); how does a device keep its merge base? (settled: D29)
- [x] Q35 (decision) — Canvas id clashes: new canvas ids (`text`, `text_2`) can collide across offline devices; how are ids made unique? (settled: D30)

## Fog

## Accepted unknowns
- Push lateness without a UnifiedPush distributor (D4): up to 15 minutes; documented, not engineered around.
- De-duplication of live and catch-up device events (D7): keyed on provider row ids; the exact keys are an implementation detail.
- Server crates building for Android aarch64 (D18): tantivy and bundled rusqlite are known to build for Android; confirmed by the first implementation ticket rather than more research.
- Range requests and streaming uploads through the custom scheme (D26): Tauri scheme handlers can return partial content; verified during implementation.
- In-process requests act as the device owner (D26): no session or token is needed between the webview and its own server; the device token is used only towards the home server.
- Conflict copies link to their original note (D20): the link shape is left to the spec.
- Load from tens of thousands of message notes (D15): covered by the deferred optimisation pass and its benchmarks.
- RCS name matching can pick the wrong person when names repeat (D24): accepted because RCS capture is already marked partial (D13).
- Android asset cache size limit and eviction policy (D19): any sensible default with a setting; the spec does not constrain it.

## Out of scope
- Windows, macOS and iOS clients.
- Building anything during mapping.
