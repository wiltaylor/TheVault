---
id: "20"
decision: D18
---

# Front-end delivery under offline editing

## Question
Remote-loaded UI or bundled UI with a local replica?

## Answer
The app runs the server stack (api, writer, index, store, state, script) in-process against a local copy of the vault, serves the bundled `web/dist` and `/api` from it, and syncs that copy with the home server. Supersedes D1.

## Why
Reuses validation, search, queries and scripts unchanged; the web code stays same-origin; device scripts get a real offline `vault`.

## Consequences
The transport between webview and local server must not be reachable by other apps (Q30). The server crates must build for Android (aarch64) and Linux desktop. The UI version now ships with the app, so the app and home server can differ in version: the sync protocol needs a version check (Q31).
