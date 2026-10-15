---
id: "13"
decision: D11
---

# Linux packaging

## Question
Which Linux packages?

## Answer
AppImage and an AUR `thevault-bin` package from GitHub Releases, plus the deb and rpm Tauri builds anyway. No Flatpak.

## Why
The user runs CachyOS (Arch); AppImage runs anywhere; Flatpak sandboxing complicates the global hotkey and the keyring.

## Consequences
Release CI builds all four artefacts.
