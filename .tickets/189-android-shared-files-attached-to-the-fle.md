---
id: '189'
title: 'Android: shared files attached to the fleeting note, and the camera from the
  app''s file inputs'
plan: capture
blocked_by: []
status: todo
verified: unknown
---

# Android: shared files attached to the fleeting note, and the camera from the app's file inputs

Spec: /home/wil/dev/TheVault/.scratch/plans/capture/SPEC.md, K4 and K1's app part. Acceptance: a test reproducing the original failure (shared non-image file, and several files) now passing; images in image and other files as asset blocks, and both reaching the home server after sync (embedded or server test); the app's webview opening the camera for a capture file input if it did not (Robolectric test of the chooser intent); ./dev just android-test passes. Note what needs a real phone to confirm. Targeted tests only.

## Worklog
