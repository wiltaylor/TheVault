// The notes opened lately in this browser, most recent first, for the command palette to offer
// before anything is typed. A note counts as opened when any pane comes to show it.

import { createEffect, createRoot, createSignal } from "solid-js";
import { noteTitle, noteType } from "../editor/titles";
import * as M from "../workspace/model";
import { workspace } from "../workspace/store";

export const RECENTS_KEY = "thevault.palette.recents";

/** How many recent notes the browser keeps. */
export const MAX_RECENTS = 30;

export interface Recent {
  id: string;
  title: string;
  type: string;
  /** When it was last opened, in milliseconds since the epoch. */
  at: number;
}

function valid(x: unknown): x is Recent {
  const r = x as Recent;
  return !!r && typeof r.id === "string" && typeof r.title === "string" && typeof r.type === "string" && typeof r.at === "number";
}

function load(): Recent[] {
  try {
    const raw = JSON.parse(localStorage.getItem(RECENTS_KEY) ?? "[]");
    return Array.isArray(raw) ? raw.filter(valid).slice(0, MAX_RECENTS) : [];
  } catch {
    return [];
  }
}

const [list, setList] = createRoot(() => createSignal<Recent[]>(load()));

function save(next: Recent[]) {
  setList(next);
  try {
    localStorage.setItem(RECENTS_KEY, JSON.stringify(next));
  } catch {
    // Private browsing or a full store: the list lasts until the page closes.
  }
}

/** Reactive: the recent notes, most recent first. */
export const recents = list;

/** Read the list from the browser again, as a test that changed it does. */
export function reloadRecents() {
  setList(load());
}

/** Note `id` was opened: it moves to the front, keeping the title and type it had if none is given. */
export function recordRecent(id: string, title?: string, type?: string, at = Date.now()) {
  const old = list().find((r) => r.id === id);
  const entry: Recent = { id, title: title ?? old?.title ?? "", type: type ?? old?.type ?? "", at };
  save([entry, ...list().filter((r) => r.id !== id)].slice(0, MAX_RECENTS));
}

/** The note's title or type changed, or became known: the entry follows, in place. */
export function updateRecent(id: string, title: string, type?: string) {
  const old = list().find((r) => r.id === id);
  if (!old || (old.title === title && (type === undefined || old.type === type))) return;
  save(list().map((r) => (r.id === id ? { ...r, title, type: type ?? r.type } : r)));
}

/** Drop note `id`, as when it no longer exists. */
export function forgetRecent(id: string) {
  if (list().some((r) => r.id === id)) save(list().filter((r) => r.id !== id));
}

const NOTE = /^\/notes\/([0-9A-HJKMNP-TV-Z]{26})(?:[/?#]|$)/i;

/** The note a pane's location shows, if it shows one. */
export function noteAt(url: string): string | null {
  return NOTE.exec(url)?.[1] ?? null;
}

/**
 * Record each note a pane comes to show, and keep the entries' titles current. Call once, under
 * the component that shows the workspace.
 */
export function trackRecents() {
  const shown = new Map<string, string>();
  createEffect(() => {
    const w = workspace();
    const active = M.activePane(w);
    // The active pane last, so it is the most recent.
    const panes = Object.keys(w.panes).sort((a, b) => Number(a === active) - Number(b === active));
    for (const pane of panes) {
      const id = noteAt(M.location(w, pane));
      if (!id) {
        shown.delete(pane);
        continue;
      }
      if (shown.get(pane) === id) continue;
      shown.set(pane, id);
      recordRecent(id);
    }
    for (const pane of [...shown.keys()]) if (!w.panes[pane]) shown.delete(pane);
  });
  // Only the notes on show, whose titles load anyway, and entries still without a title: a
  // request per remembered note would be wasted.
  createEffect(() => {
    const w = workspace();
    const open = new Set(Object.keys(w.panes).map((p) => noteAt(M.location(w, p))));
    for (const r of list()) {
      if (r.title && !open.has(r.id)) continue;
      const t = noteTitle(r.id);
      if (typeof t === "string") updateRecent(r.id, t, noteType(r.id));
    }
  });
}
