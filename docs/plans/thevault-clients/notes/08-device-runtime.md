---
id: "08"
decision: D6
---

# Device script runtime

## Question
Embed wscript on the device or forward raw events?

## Answer
Embed wscript in the Android app. The script engine (Kind, Job, Limits, Host and the `Vault` trait) moves out of `crates/script` into a crate with no server-only dependencies; the device implements `Vault` over the HTTP API with its device token (and over the local replica when offline, D8). A new `device` module exposes device data.

## Why
wscript is pure Rust with no third-party dependencies; the `trait Vault` seam already exists (`crates/script/src/vault.rs:16`). On-device scripts can filter private data before upload.

## Consequences
Background triggers run without a webview (note 01), so the engine is called through JNI from Kotlin (Q20).
