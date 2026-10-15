---
id: "30"
decision: D27
---

# Version skew

## Question
What happens when app and server versions differ?

## Answer
Sync carries a protocol version and the vault format version. If the server is newer than the app supports, the app refuses to sync and asks for an update while local edits stay queued. If the app is newer, it syncs while the server's version is within its supported range. The desktop app checks GitHub Releases for updates; Obtainium updates Android.

## Why
A mismatched app writing an older format could corrupt notes; refusing while queueing loses nothing.

## Consequences
The server exposes its protocol and format versions on an unauthenticated endpoint or in the sync handshake.
