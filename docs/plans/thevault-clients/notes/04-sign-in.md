---
id: "04"
decision: D2
---

# App sign-in

## Question
How does an app get credentials?

## Answer
A one-time native setup asks for the server URL and password. The server issues a per-device token, stored in the Android Keystore or the Linux Secret Service keyring. The webview exchanges the token for a browser session, so a server restart never signs the app out. QR pairing from the web app's Settings is offered as an alternative to typing the password.

## Why
Background work (device events, push, sync) needs a credential that outlives the in-memory browser session (grounding: sessions are in memory, `crates/api/src/auth.rs:98-130`).

## Consequences
The server needs a pairing endpoint, a token-to-session exchange endpoint, and QR pairing codes with a short lifetime.
