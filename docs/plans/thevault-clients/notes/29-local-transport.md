---
id: "29"
decision: D26
---

# Webview to in-process server transport

## Question
Custom scheme or localhost?

## Answer
A custom URI scheme handled in-process, routed into the existing axum router; no network port. Requests through it act as the device owner.

## Why
Any app on the phone can reach localhost; a custom scheme is reachable only from the app's own webview.

## Consequences
Range requests (pdf.js) and streaming uploads must work through the scheme handler.
