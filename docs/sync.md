# Device sync on the server

The desktop and Android apps keep a copy of the vault and sync it with the home server, which holds the master copy (D20). This page describes what the server provides for that sync and what it enforces. The sync engine on the device is in `crates/sync`.

## Endpoints

| Endpoint | Credentials | What it does |
|---|---|---|
| `GET /api/sync/info` | none | Returns `{protocol, format_version, server_version}`. An app refuses to sync, and keeps its queued writes, when either version is newer than it supports (D27). |
| `GET /api/version` | none | Returns `{server_version}`, for Settings > General's About section. |
| `GET /api/sync/changes?since=<seq>&limit=<n>` | any token or session | Returns every note and synced file whose latest change comes after `since`, oldest first, with `next`, `latest` and `remaining`. |
| `PUT /api/devices/{id}/bases` | the device's own token, or `admin` | Replaces the base versions the device holds: `{"bases": {"<note id>": "<version>"}}`. |

A device pushes its own changes through the normal notes API, with the base version in `If-Match`. Two more writes keep the ids a device made offline (D19, D20):

| Endpoint | Credentials | What it does |
|---|---|---|
| `PUT /api/notes/{id}?type=<type>` with `If-None-Match: *` | `write` | Creates the note under `id`, which must be a ULID no note has ever had, live, in the trash or in the history. Otherwise 409 `id_taken`. |
| `PUT /api/assets/{id}?filename=<name>` | `write` | Stores the body as asset `id`. The same file again answers 200; anything else under the id is 409 `id_taken`. |

## The change feed

Each note and each synced file appears in the feed at most once, at its latest change. Each change gets a new `seq` that is larger than every earlier one. A device stores the `next` of each page and asks for the next page from it. The device is up to date when `next` equals `latest`. `remaining` is how many entries come after the page, so a device knows from the first page how many entries its pull reads, which its progress bar shows; `latest - next` would overstate it, since a key that changes again leaves a gap in `seq`. `since` defaults to 0, which returns everything. `limit` defaults to 500 and can be at most 5000.

Each entry has `seq`, `at` (milliseconds since the epoch) and `kind`. It shows the note or file as it is when the page is read:

- `kind: "note"`: `id`, `type`, `version` and `wcl`, with `deleted: false` and `trashed: false`. For a deleted note, `deleted` is `true`, `wcl` is `null`, and `trashed` is `true` when the note is in the trash, with its `type` and the `version` it had when it was deleted. Restoring a note from the trash puts it in the feed again as a live note.
- `kind` is `type`, `taxonomy`, `script`, `task` or `device` for the files under `types/`, `taxonomies/`, `scripts/`, `tasks/` and `devices/`: `path`, `name` and `text`, or `deleted: true` with `text: null`. Templates are part of their type file, so a template change appears as a change to its type.

Assets are not in the feed. A device fetches them when it needs them (D19).

Changing a note always moves it in the feed, including a browser save that merges into the version before it. Files are journaled after each commit. The server also checks the files on disk each time it opens the vault. That check covers files that existed before the feed did, files added outside the server, and a crash between a commit and its journal entry.

## Versions and bases

The server enforces these rules for writes and history (D29):

- **A device write is always a new version.** Writes made with a token never merge into the version before them. A device token is a token, so each device write gets a version of its own. The browser's next save does not merge into a device's version either, because only a session's saves merge into that same session's version.
- **A recorded base is kept.** Pruning never removes a version that a device lists in its bases. A browser save does not merge into such a version, because merging would rewrite it. Once the device leaves the version out of its bases, or the device is revoked, pruning can remove it like any other version.
- **A base must exist.** Each base must be an existing version of its note. Otherwise the request fails with 400 `unknown_version` and the device's previous bases stay in place.

## What a device token may write

A device pairs with a `write` token (D2). With it, a device:

- creates, updates and deletes notes, restores them from the trash, uploads assets, and writes pins and notifications, like any `write` token;
- sets the push endpoint and bases of its own device only;
- **cannot** write types, templates, taxonomies, scripts or tasks. All of those need `admin`, so the server answers 403. The device reads them, and receives them in the feed, like any other token. Definitions are edited online through a session or an `admin` token, and never through device sync (D20).

## What the apps answer locally

Both apps run the server's `/api` router in-process (`crates/embedded`, D18), against the local copy of the vault and a local `state.db`. A route whose data is a vault file, or is computed from the vault files, is answered locally and works offline. A route backed by state that only the home server's `state.db` holds is not: the local `state.db` has none of it, so before this split Settings > Devices in the apps showed no devices at all. Once the app is paired (`Embedded::set_remote`), the embedded server sends those routes to the home server with the device token, streaming request and response bodies both ways and keeping the status code and headers. The desktop app sets the remote when its sync engine starts; Android sets it whenever Kotlin hands the core its pairing (`Core::set_link`). Requests the Android webview sends over IPC (`api_request`) reach the same `Embedded::handle`, so they are forwarded too.

When a forwarded route cannot be answered, the embedded server says why instead of answering with an empty list:

| Answer | When | The web client shows |
|---|---|---|
| 503 `{"error": "offline"}` | The home server cannot be reached: the connection fails within 5 seconds, or a proxy answers 502 to 504 without JSON. | "Needs a connection to your server" |
| 503 `{"error": "not_paired"}` | The app is not paired, or the server answers 401: the device was revoked. | "Not paired" |
| 403 `{"error": "needs_web_app"}` | The route needs `admin`, which a device token never has (D2). The server's own message is in `detail`. | "Manage this in the web app" |

The device token is never widened to `admin`. In the apps, Settings hides the Secrets and API tokens sections, and Settings > Devices lists the devices read-only, marked "This device" for the app's own, and says to pair, rename, revoke and change settings in the web app. `GET /api/app`, answered only by the embedded server, tells the web client it runs in an app, which device it is, and which app build: `version` (from the app's `tauri.conf.json`), `platform` (`linux-appimage`, `linux-package`, `linux` or `android`) and `build` (`release` or `debug`). The home server answers it 404.

`GET /api/app/sync`, also answered only by the embedded server, says where the app's sync stands for the sidebar's sync icon: `state` (`synced`, green, when the last round succeeded and nothing waits; `syncing`, yellow, while a round runs or local writes wait for one; `error`, red, otherwise), `phase` (`unpaired`, `starting`, `syncing`, `synced`, `offline`, `update_required`, `updating` while sync waits for the app to update itself to a newer server's version, `revoked` or `error`), `queued` (local writes waiting), `held` (of those, the notes held back for an asset that has not reached the server), `assets_waiting` and `stuck_assets` (the assets that failed to upload, each `{id, filename, size, error, attempts, last_ms, next_ms}`), `last_ok_ms` (when a round last succeeded), `last_attempt_ms` (when the last round ended, succeeded or not), `last_error` (why the last round failed, or what a round that finished could not do, such as an asset that would not upload; the popover shows it whenever it is set), `server`, `warning` (the version gate's warning that the server is newer than the app and no matching app build is published, else null) and `progress` (how far the current or the last round got, null before the first; see "Progress" below). The web client asks for it every few seconds, every second while yellow, and at once after each note write. `POST /api/app/sync` asks the app's sync loop for a round now and answers 202 with the status, or 503 `not_paired` when the app has no sync loop.

`GET /api/app/device-events`, answered only by the Android app's embedded server, lists the device events that failed for good or were skipped, newest first, for Settings > Devices > This device: `{ device, failed, events: [{ id, event, at, status, attempts, message, who }] }`, where `device` says whether a device runtime runs and `failed` counts the failed events. `POST /api/app/device-events/<id>/retry` gives a failed event three more passes, runs the first at once and answers the event as it then stands with the new `failed` count; `POST /api/app/device-events/<id>/dismiss` hides it. Both answer 404 for an event that is not failed (docs/scripts.md, "Device events").

### Every route

| Routes | Backed by | Answered |
|---|---|---|
| `/api/notes` and below (read, create, update, delete, graph, archive, canvas), `/api/batch`, `/api/upsert`, `/api/daily` | Note files | Locally. Writes are queued for sync. |
| `/api/query`, `/api/query/parse`, `/api/query/text`, `/api/search`, `/api/flagged`, `/api/schema` | The local index of the note files | Locally |
| `/api/types`, templates, `/api/taxonomies`, `/api/scripts`, `/api/device-files` | Definition files | Locally. Edits made in an app stay on the device until the server's version replaces them (D20). |
| `GET /api/tasks/{name}`, `PUT` and `DELETE /api/tasks/{name}`, `GET /api/cron` | Task files; the cron preview is a computation | Locally |
| `/api/assets` and below | Asset files | Locally. Android fetches an asset it does not hold from the server first (D19). |
| `/api/trash` and its restore | The local trash, kept in step by the change feed's deletes | Locally |
| `/api/pins` and below | The local `state.db` | Locally. Pins are kept per device and are not synced. |
| `/api/login`, `/api/logout`, `/api/health`, `/api/openapi.json`, `/api/sync/info`, `/api/sync/changes` | The embedded server itself | Locally |
| `/api/devices` and below: the list, rename and settings, revoke, pairing, pair codes, push endpoint, capture health, bases, device commands and their results | The server's `devices`, `pair_codes`, `device_bases` and `device_commands` tables | Forwarded. Changing a device or making a pair code needs `admin`: 403 `needs_web_app`. |
| `/api/notifications` and below: list, add, read, read all, dismiss, clear read | The server's `notifications` table | Forwarded, so the app's bell matches the web app's |
| `GET /api/events` | The server's live events | Forwarded and merged, see below |
| `GET /api/tasks/{name}/runs`, `GET /api/tasks/{name}/runs/{id}` | The server's `task_runs` table | Forwarded |
| `POST /api/tasks/{name}/run` | The scheduler, which runs only on the server (D17) | Forwarded. It needs `admin`: 403 `needs_web_app`. |
| `GET /api/tasks` | Task files, with each task's last and next run from the server | Forwarded; the local list, with no runs, while offline or unpaired |
| `GET /api/notes/{id}/history` | The server's `versions` table | Forwarded, with `source: "server"`; see below |
| `GET /api/notes/{id}/history/{version}` | The version's own copy | Locally when the device recorded that version, otherwise forwarded |
| `POST /api/notes/{id}/restore` | A note write | Locally; see below |
| `/api/clients` and below | The server's clients folder | Forwarded, `Range` included |
| `GET /api/version` | The home server's version | Answered from the home server's `GET /api/sync/info`, so an older server still says its version; offline or unpaired, 503 |
| `/api/tokens`, `/api/secrets` and below | The server's `tokens` and `secrets` tables, which need `admin` even to read | Not forwarded: 403 `needs_web_app` at once |

### The history

The home server's history is authoritative: it holds every version from every device and the browser, while a device records only the versions it wrote or pulled, each under a version id of its own. So the History panel lists the server's versions while the app is online. Offline, unpaired, or for a note the server does not have yet, it lists the versions the device recorded, with `source: "device"`, and the panel says it shows only this device's versions.

Restoring is always a local write, so it works like any edit and syncs. A version the device recorded is restored as on the server. A version only the server holds is read from the server as WCL and written locally as a new version over the request's base version, so a stale base is still a 409 conflict.

### The event stream

`GET /api/events` through the embedded server streams the home server's events with the device's own `note.changed` in place of the server's: a note changes on the device when the device writes or pulls it, under a version id of its own, so the server's `note.changed` would name versions the device does not hold. SSE ids are left out, since the two sources do not count together. The stream ends when the server's ends, and offline the request answers 503.

The app shells answer their webview's custom scheme with whole bodies (Tauri cannot stream a custom scheme response), so both shells answer `/api/events` with 503 `no_stream` at once rather than hold a stream to the home server open that could never be answered. The web client then polls the bell every 30 seconds and on focus, as it does whenever the stream is down. The desktop app's tray and the Android app's sync each follow the home server's stream themselves.

### Notifications made on the device

A device script's `vault::notify` writes to the local `state.db`. Each sync round moves those notifications to the home server with `POST /api/notifications`, after pushing the notes they may open, and removes them locally, so they show in every bell.

## The sync engine on the device

`crates/sync` is the device side, used by both apps through `crates/embedded`. Its crate documentation describes one round step by step; in short:

- **The queue.** Every local note write is queued in the local `state.db` (tables `sync_*`), one row per note in the order of its first change, with the base it started from: the server version and its WCL text. A note changed again before it syncs keeps its first base. What the pull writes is never queued.
- **Handshake.** A round starts with `GET /api/sync/info` and stops, keeping the queue, when the server's protocol or format version is newer than the app's (D27).
- **Push.** Assets the device stored go first, then each queued note in order. An upload streams from the file with no total time limit and fails only when no bytes move for 60 s (the connect timeout is 15 s, a JSON request's 60 s). An asset that fails does not fail the round: it is recorded, tried again by later rounds after 30 s, doubling to at most an hour (at once after "Sync now"), and the queued notes that name it wait, since the server refuses a note naming an asset it does not have; the other notes go on. Only a server that cannot be reached, or refuses the token, fails the round there. A 409 starts a three-way merge of base, device and server: fields one by one, and the top-level blocks of a content field by a diff3 over each block's canonical text. A clash, or blocks that cannot be aligned (a block repeated in the base, or blocks moved on one side), keeps the server's version and makes a conflict copy titled `<title> (conflict, <device name>)` whose built-in `conflict_of` field names the original (see `docs/vault-format.md` "Conflict copies"), and a notification that opens it (D20, D28). A device's delete loses to a server change made since its base; a device's change to a note the server deleted restores the note first.
- **Pull.** The change feed from the stored cursor, a page at a time, written to the local vault exactly as the server holds it and indexed. A note with a change still queued waits for the next push to merge it.
- **Bases.** The engine records the base of every queued change with `PUT /api/devices/{id}/bases` at the start and the end of each round (D29).
- **Assets.** The desktop keeps every asset. Android fetches an asset when a note needs it, prefetches the assets of pinned and recently opened notes, and evicts the least recently used ones beyond its cache size; a note may name an asset the device does not hold while the server does (D19).
- **Definitions** are never queued, so types, templates, taxonomies, scripts and tasks changed on a device stay there until the server's version replaces them (D20).

### Progress

Each round keeps a progress report up to date, which `GET /api/app/sync` answers as `progress` and the desktop's `GET /_desktop/status` as `progress` too (ticket 144):

| Field | Meaning |
| --- | --- |
| `stage` | `handshake`, `pushing`, `pulling`, `fetching`, then `done` or `failed`. |
| `running` | Whether the round still runs. |
| `notes_done`, `notes_total` | Feed entries, notes and definition files, applied out of those the pull reads: what it has read, plus the page, plus the page's `remaining`. A server too old to send `remaining` gives `latest - next` instead, an overestimate. |
| `files_done`, `files_total` | Asset files fetched out of those to fetch. The pull starts with `GET /api/assets`, so the desktop knows them, every asset the server lists that it does not hold, from the start; Android knows them once it has worked out which assets its pins and recent notes need. |
| `bytes_done`, `bytes_total` | Asset bytes, counted as each block of a download arrives, out of the sizes `GET /api/assets` lists. |
| `eta_secs` | About how many seconds are left, null until there is a rate to go by. |
| `elapsed_ms` | How long the round has run. |
| `large` | Whether the round reads more than 200 feed entries or fetches more than 20 MB. |

Every count and total only grows during a round; the next round starts from zero.

The time left is measured, not guessed from sizes alone. About four times a second the engine keeps a sample of the counts, and the rates are those over the last 10 seconds of the current stage: feed entries a second and asset bytes a second. The time left is the entries left over the entries rate, plus the bytes left over the bytes rate, rounded to the nearest second. While the pull runs and no asset byte has arrived, the bytes rate is that of the feed pages themselves, the best guess at how fast the network moves data. There is no estimate until a rate has been measured over at least a second.

The sync popover shows a bar and a line such as `Notes 1200 / 1200 · Files 3 / 10 (12.0 / 40.0 MB) · about 2 min left` while a `large` round runs, so a phone's first sync shows it as the desktop's setup page does.

## When the Android app syncs

The Android app runs one Rust core per process, shared by its webview and by the Kotlin code that runs with no webview (`plugins/tauri-plugin-vault-device`). The core holds the pairing in memory. Kotlin stores the server address, device id and name in preferences, and keeps the token sealed under a key in the Android Keystore (D2). A round runs:

- when the app opens or returns to the foreground;
- 1.5 seconds after the last local write, including a share from another app;
- while the app is in the foreground, 1 second after the home server's event stream (`GET /api/events`, followed with the device token) reports a `note.changed` for a version the phone does not hold, 1 second after it reports a `device.settings` for this phone, and 1 second after the stream (re)connects. The stream reconnects with a pause that grows from 1 to 30 seconds, and closes within a heartbeat (15 seconds) once the app leaves the foreground. The echo of the phone's own pushed write is not news and runs no round;
- in the background, when a `note.changed` or `device.settings` push arrives over UnifiedPush: the app schedules the same prompt round a background capture schedules (`SyncWorker.soon`);
- every 15 minutes in the background through WorkManager, while the phone has a network. A server that cannot be reached is retried later. A server newer than the app stops background sync until the app is updated (D27).

Each round first takes the phone's settings from Settings > Devices (`GET /api/devices`): its contacts write and SMS send switches, its enabled commands, its exclusion list and its skip pattern (D22, D23, D25). So a setting changed in the web app applies on the phone by its next round, within seconds while the app is open or a push reaches it, and within 15 minutes otherwise.

With the settings, the round sends the phone's capture health to `PUT /api/devices/{id}/health` when it changed since the server last took it (ticket 91). The body is `{ "problems": [...], "kinds": [...], "battery_optimised": ..., "background": [...] }`: `problems` lists what stops capture in words, such as `SMS: permission missing`, `Battery optimisation is on` or `Catch-up of messages, calls and contacts has not run for 3 hours`, and is empty when all is well; `kinds` gives each capture kind's state (`working`, `permission_missing`, `restricted` or `not_set_up`); `background` says whether the catch-up and the command check are stalled. Only the device's own token, or an admin, may send it; a body that is not an object with `problems`, a list of at most 32 strings of 1 to 300 characters, or that is over 16 KiB, answers 400. `GET /api/devices` then carries it as each device's `health`, with `reported`, when the server heard it, or `null` before the first report, and the web app's device card shows a warning line while `problems` is not empty.

After each round, a `person` the round changed queues a `contact_changed` event for its linked phone contact, so a rename made in the web app reaches the contact (D16). While contacts write is off, nothing is queued and nothing is marked as seen, so the people changed in the meantime reach their contacts in the first round after it is turned on.

### The `note.changed` push

The server's push dispatcher (`crates/push`) sends each device with a UnifiedPush endpoint `{"event": "note.changed", "ids": [...]}` when notes change, with `Urgency: normal`. It coalesces: the first change after a quiet spell goes out 1 second later, and no device gets more than one `note.changed` every 10 seconds; the changes in between go out together. A device is never told about the changes its own token made, so a device whose own writes are all that changed gets no push. At most 64 ids go in one message; a longer burst adds `"more": true`. The ids are a hint: the device pulls the change feed either way.

### The `device.settings` push

Changing a device's settings (`PATCH /api/devices/{id}` with `settings`) publishes `device.settings` with `{device}` on `GET /api/events`, and pushes `{"event": "device.settings"}` to that device alone, with `Urgency: normal`, coalesced as `note.changed` is: a burst of switches sends one push. A rename alone sends nothing.

Only one round runs at a time. When a note shows an asset the phone does not hold, the app fetches it from the server before answering. Opening a note records it, so the next round prefetches its assets. The asset cache defaults to 512 MiB, and onboarding sets its size.
