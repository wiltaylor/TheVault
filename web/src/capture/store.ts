// Whether the capture dialog is open, and the inbox's count of fleeting notes waiting.

import { createSignal } from "solid-js";
import { api } from "../api";
import { onNoteWritten } from "../changes";
import { INBOX_QUERY } from "./model";

const [open, setOpen] = createSignal(false);

/** Reactive: whether the capture dialog shows. */
export const captureOpen = open;

/** Show the capture dialog, from anywhere in the app. */
export function openCapture() {
  setOpen(true);
}

export function closeCapture() {
  setOpen(false);
}

/** Whether a key press is the capture shortcut, Ctrl/Cmd+Shift+K. */
export function isCaptureKey(e: KeyboardEvent): boolean {
  return (e.ctrlKey || e.metaKey) && e.shiftKey && !e.altKey && e.code === "KeyK";
}

const [count, setCount] = createSignal<number | null>(null);

/** Reactive: how many fleeting notes wait in the inbox, once counted. */
export const inboxCount = count;

let counting = 0;

/** Count the inbox again. Only the latest count is kept when several overlap. */
export async function refreshInboxCount(): Promise<void> {
  const mine = ++counting;
  try {
    const page = await api.query(INBOX_QUERY, null, 1);
    if (mine === counting) setCount(page.total);
  } catch {
    // A vault without the type, or a failed request, shows no badge.
    if (mine === counting) setCount(null);
  }
}

onNoteWritten(() => void refreshInboxCount());
