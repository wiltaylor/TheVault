---
id: "19"
decision: D17
---

# Background execution on Android

## Question
How do device scripts run with no window open?

## Answer
Kotlin receivers and WorkManager workers call into the app's Rust core through JNI. Device events land in a durable queue in the local `state.db`. Device scripts run against the local vault copy and their writes sync like any edit. Scheduled tasks and after-save handlers run only on the home server; validation runs locally.

## Why
The webview and Tauri runtime may not exist in the background (note 01). Running tasks and handlers only on the server prevents double execution.

## Consequences
The local server stack needs a mode with scheduler and handler queue disabled.
