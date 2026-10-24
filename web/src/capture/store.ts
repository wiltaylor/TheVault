// Whether the capture dialog is open, and the inbox's count of fleeting notes waiting.

import { createSignal } from "solid-js";
import { ASSISTANT, callableAgents } from "../agents";
import { type AgentSummary, api } from "../api";
import { onNoteChangedElsewhere, onNoteWritten } from "../changes";
import { FLEETING, INBOX_QUERY } from "./model";

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

/** Where the capture dialog's Send to choice is kept, per device (capture K3). */
export const SEND_TO_KEY = "thevault.capture.send-to";

/** The agent the last capture was sent to, or "" for No one. */
export function lastSendTo(): string {
  try {
    return localStorage.getItem(SEND_TO_KEY) ?? "";
  } catch {
    return "";
  }
}

export function rememberSendTo(agent: string) {
  try {
    if (agent) localStorage.setItem(SEND_TO_KEY, agent);
    else localStorage.removeItem(SEND_TO_KEY);
  } catch {
    // Without storage the choice is not kept.
  }
}

/**
 * The agents a capture may be sent to, in the Send to list's order: Assistant, then every other
 * enabled agent a fleeting note may call (agents G16). Before the list loads, as when the app
 * has no connection, Assistant alone: its call is answered once the note syncs.
 */
export function sendToAgents(list: readonly AgentSummary[]): Pick<AgentSummary, "name" | "label">[] {
  if (!list.some((a) => a.name === ASSISTANT)) return [{ name: ASSISTANT, label: "Assistant" }, ...callableAgents(list, FLEETING, "")];
  const callable = callableAgents(list, FLEETING, "");
  return [...callable.filter((a) => a.name === ASSISTANT), ...callable.filter((a) => a.name !== ASSISTANT)];
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
// A capture or a processed note on another device, or by an agent, moves the count too.
onNoteChangedElsewhere(() => void refreshInboxCount());
