---
id: "06"
decision: D4
---

# Push channel

## Question
How do server events reach a phone whose app is closed?

## Answer
UnifiedPush through a distributor the user runs (for example ntfy, self-hostable), via our own small Kotlin plugin on `org.unifiedpush.android:connector`. Without a distributor the app falls back to a WorkManager check every 15 minutes. The server also gains a live event stream (SSE) that the web app, the desktop app and an open Android app use instead of 30-second polling. Recorded as recommended after research note 01; the foreground-service fallback was dropped because of Tauri issue #15671.

## Why
Self-hosted friendly, no Google dependency (keeps F-Droid possible, note 02), and the only option that clears notifications within seconds.

## Consequences
Server needs UnifiedPush delivery (WebPush-encrypted POST to the device's endpoint) and an SSE endpoint. Without a distributor, cross-device notification clearing is up to 15 minutes late.
