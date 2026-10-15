---
id: "14"
decision: D12
---

# Desktop-only features

## Question
Which desktop features?

## Answer
Tray icon with unread badge and Quick capture; desktop notifications mirrored from the server; start on login minimised; a global capture hotkey through the XDG global-shortcuts portal on Wayland (falling back to a user-bound `thevault --capture`); `thevault://` links that open a note.

## Why
All recommended; cheap with Tauri plugins.

## Consequences
Wayland desktops without the portal need the manual binding, documented.
