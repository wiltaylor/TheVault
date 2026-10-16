---
id: '85'
title: Pins are kept per device and do not sync between the web app, desktop and Android
  apps
plan: thevault-clients
blocked_by: []
status: todo
verified: unknown
---

# Pins are kept per device and do not sync between the web app, desktop and Android apps

Found while making the apps forward home-server-only routes (commit 15d7967 era): pins live in each device's local state, so pinned notes differ per device, and Android asset prefetch uses local pins. Decide whether pins belong in vault files (synced) or on the home server (forwarded), and make all clients agree.

## Worklog
