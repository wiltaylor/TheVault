# Device sync on the server

The desktop and Android apps keep a copy of the vault and sync it with the home server, which holds the master copy (D20). This page describes what the server provides for that sync and what it enforces. The sync engine on the device is in `crates/sync`.

## Endpoints

| Endpoint | Credentials | What it does |
|---|---|---|
| `GET /api/sync/info` | none | Returns `{protocol, format_version, server_version}`. An app refuses to sync, and keeps its queued writes, when either version is newer than it supports (D27). |
| `GET /api/sync/changes?since=<seq>&limit=<n>` | any token or session | Returns every note and synced file whose latest change comes after `since`, oldest first, with `next` and `latest`. |
| `PUT /api/devices/{id}/bases` | the device's own token, or `admin` | Replaces the base versions the device holds: `{"bases": {"<note id>": "<version>"}}`. |

A device pushes its own changes through the normal notes API, with the base version in `If-Match`.

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
