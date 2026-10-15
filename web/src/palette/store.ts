// Whether the command palette is open, and its shortcut.

import { createSignal } from "solid-js";

const [open, setOpen] = createSignal(false);

/** Reactive: whether the palette shows. */
export const paletteOpen = open;

export function openPalette() {
  setOpen(true);
}

export function closePalette() {
  setOpen(false);
}

const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

function editing(el: EventTarget | null): boolean {
  return el instanceof HTMLElement && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));
}

/**
 * Whether a key press is the palette shortcut, Ctrl/Cmd+K. On a Mac, Ctrl+K in text deletes to
 * the end of the line, so there only Cmd+K opens the palette from text.
 */
export function isPaletteKey(e: KeyboardEvent, mac = isMac): boolean {
  if (e.shiftKey || e.altKey || e.code !== "KeyK") return false;
  if (e.metaKey) return true;
  return e.ctrlKey && !(mac && editing(e.target));
}

/** The palette shortcut as the app shows it. */
export const PALETTE_KEYS = "Ctrl+K";
