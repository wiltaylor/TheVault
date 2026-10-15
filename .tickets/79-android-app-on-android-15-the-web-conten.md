---
id: '79'
title: 'Android app: on Android 15 the web content draws under the status bar (edge-to-edge),
  so the header and search box sit under the clock and icons'
plan: thevault-clients
blocked_by: []
status: done
verified: unmet
---

# Android app: on Android 15 the web content draws under the status bar (edge-to-edge), so the header and search box sit under the clock and icons

Found by the emulator run of docs/android-manual-checks.md (Android 15 API 35 emulator, Pixel 6 profile; targetSdk 36).

Repro: open the app on Android 15 or later.

Expected: the app's header (menu button, search box, bell) below the status bar.

Actual: Android 15 enforces edge-to-edge for apps targeting SDK 35+, and the WebView fills the whole window, so the status bar's clock and icons draw over the menu button, the search box and the bell (see screenshot). Taps near the top edge can land on the status bar.

Fix direction: apply the system bar insets, e.g. WindowInsets padding on the WebView in MainActivity / the Tauri activity, or viewport-fit=cover with env(safe-area-inset-top) in the web client for the app.

Evidence: .scratch/emulator/evidence/status-bar-overlap.png (any screenshot under .scratch/emulator/shots shows it).

## Worklog

### 2026-10-15T11:50:16Z — wil

Status set to claimed.

### 2026-10-15T11:50:17Z — wil

Fixed in 9e06d08: the plugin's SystemBars passes status, navigation, cutout and IME insets to the page as --sat/--sar/--sab/--sal and raises the WebView above the keyboard; the shell, drawers, + button, toasts and sheets pad with max(env(safe-area-inset-*), var(--sa*)); status bar icons follow the theme. check, android-test and the touched Playwright specs pass; browsers unchanged. Emulator confirmation pending.

### 2026-10-15T11:50:17Z — wil

Verified set to unmet.

### 2026-10-15T11:50:17Z — wil

Status set to done.
