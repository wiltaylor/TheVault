---
id: '135'
title: release.yml builds the AppImage without TAURI_SIGNING_PRIVATE_KEY, so a CI
  release (and the image built from it) carries no updater signature and installed
  desktop apps will not update to it; pass the updater key as a repository secret
  and attach the .AppImage.sig
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# release.yml builds the AppImage without TAURI_SIGNING_PRIVATE_KEY, so a CI release (and the image built from it) carries no updater signature and installed desktop apps will not update to it; pass the updater key as a repository secret and attach the .AppImage.sig

## Worklog
