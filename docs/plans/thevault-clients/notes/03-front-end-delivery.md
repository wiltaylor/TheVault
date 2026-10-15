---
id: "03"
decision: D1
---

# Front-end delivery

## Question
Does the app's webview load the server URL, or bundle `web/dist`?

## Answer
The webview loads the server's URL. A small bundled native page handles first-run setup and the server-unreachable state. Native abilities are granted to the remote page through a Tauri 2 remote capability scoped to the configured server origin.

## Why
The UI always matches the server version, so there is no client/server skew, and the web code needs no base-URL, CORS or asset-auth changes (grounding: relative `/api` paths, `SameSite=Strict` cookie, no CORS).

## Consequences
D8 (full offline editing) conflicts with a remote-loaded UI: the UI and data must be available with no server. Q21 revisits this decision under D8.
