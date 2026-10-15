# Device sync on the server

The desktop and Android apps keep a copy of the vault and sync it with the home server, which holds the master copy (D20). This page describes what the server provides for that sync and what it enforces. The sync engine on the device is in `crates/sync`.

## Endpoints

| Endpoint | Credentials | What it does |
|---|---|---|
| `GET /api/sync/info` | none | Returns `{protocol, format_version, server_version}`. An app refuses to sync, and keeps its queued writes, when either version is newer than it supports (D27). |
| `GET /api/sync/changes?since=<seq>&limit=<n>` | any token or session | Returns every note and synced file whose latest change comes after `since`, oldest first, with `next` and `latest`. |
| `PUT /api/devices/{id}/bases` | the device's own token, or `admin` | Replaces the base versions the device holds: `{"bases": {"<note id>": "<version>"}}`. |

A device pushes its own changes through the normal notes API, with the base version in `If-Match`. Two more writes keep the ids a device made offline (D19, D20):

| Endpoint | Credentials | What it does |
|---|---|---|
| `PUT /api/notes/{id}?type=<type>` with `If-None-Match: *` | `write` | Creates the note under `id`, which must be a ULID no note has ever had, live, in the trash or in the history. Otherwise 409 `id_taken`. |
| `PUT /api/assets/{id}?filename=<name>` | `write` | Stores the body as asset `id`. The same file again answers 200; anything else under the id is 409 `id_taken`. |

## The change feed

Each note and each synced file appears in the feed at most once, at its latest change. Each change gets a new `seq` that is larger than every earlier one. A device stores the `next` of each page and asks for the next page from it. The device is up to date when `next` equals `latest`. `since` defaults to 0, which returns everything. `limit` defaults to 500 and can be at most 5000.

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

## The sync engine on the device

`crates/sync` is the device side, used by both apps through `crates/embedded`. Its crate documentation describes one round step by step; in short:

- **The queue.** Every local note write is queued in the local `state.db` (tables `sync_*`), one row per note in the order of its first change, with the base it started from: the server version and its WCL text. A note changed again before it syncs keeps its first base. What the pull writes is never queued.
- **Handshake.** A round starts with `GET /api/sync/info` and stops, keeping the queue, when the server's protocol or format version is newer than the app's (D27).
- **Push.** Assets the device stored go first, then each queued note in order. A 409 starts a three-way merge of base, device and server: fields one by one, and the top-level blocks of a content field by a diff3 over each block's canonical text. A clash, or blocks that cannot be aligned (a block repeated in the base, or blocks moved on one side), keeps the server's version and makes a conflict copy titled `<title> (conflict, <device name>)` whose built-in `conflict_of` field names the original (see `docs/vault-format.md` "Conflict copies"), and a notification that opens it (D20, D28). A device's delete loses to a server change made since its base; a device's change to a note the server deleted restores the note first.
- **Pull.** The change feed from the stored cursor, a page at a time, written to the local vault exactly as the server holds it and indexed. A note with a change still queued waits for the next push to merge it.
- **Bases.** The engine records the base of every queued change with `PUT /api/devices/{id}/bases` at the start and the end of each round (D29).
- **Assets.** The desktop keeps every asset. Android fetches an asset when a note needs it, prefetches the assets of pinned and recently opened notes, and evicts the least recently used ones beyond its cache size; a note may name an asset the device does not hold while the server does (D19).
- **Definitions** are never queued, so types, templates, taxonomies, scripts and tasks changed on a device stay there until the server's version replaces them (D20).
