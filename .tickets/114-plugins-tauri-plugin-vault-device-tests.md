---
id: '114'
title: plugins/tauri-plugin-vault-device/tests/core.rs is not rustfmt-clean (cargo
  fmt --check in apps/android/src-tauri shows a diff near line 430), and no gate checks
  the plugin crate's formatting
blocked_by: []
status: done
verified: verified
---

# plugins/tauri-plugin-vault-device/tests/core.rs is not rustfmt-clean (cargo fmt --check in apps/android/src-tauri shows a diff near line 430), and no gate checks the plugin crate's formatting

## Worklog

### 2026-10-22T02:37:20Z — wil

Claimed.

### 2026-10-22T02:55:28Z — wil

Done on ticket/114 (6a5e439). The plugin and both app crates were already rustfmt-clean on main. just check (ci::check) now runs ci::apps-fmt-check (cargo fmt --check in apps/android/src-tauri incl. the plugin, and apps/desktop/src-tauri) and ci::apps-lint (host clippy -D warnings on both app workspaces, plus scripts/android-check.sh: cargo ndk x86_64 clippy over store, embedded and the Android workspace, about 25 s cold). just fmt formats both app workspaces. Fixed the plugin commands test broken by ticket 115 (run ?wait=true). Dropped the stdin file workaround in crates/cli/skill/thevault/test.py. ./dev just check and ./dev just android-test pass.

### 2026-10-22T02:55:35Z — wil

Verified set to verified.

### 2026-10-22T02:55:35Z — wil

Status set to done.
