// The live workspace: one signal holding the pure model, saved to localStorage on every change,
// with the browser's address following the active pane.

import { createRoot, createSignal } from "solid-js";
import * as M from "./model";

/** The browser keeps the workspace under this key. */
export const WORKSPACE_KEY = "thevault.workspace";

const [ws, setWs] = createRoot(() => createSignal<M.Workspace>(M.emptyWorkspace()));

/** Reactive: the workspace as it stands. */
export const workspace = ws;

let saveTimer: ReturnType<typeof setTimeout> | undefined;

function save() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(saveNow, 150);
}

/** Write the workspace to localStorage now, rather than after the next pause. */
export function saveNow() {
  clearTimeout(saveTimer);
  try {
    localStorage.setItem(WORKSPACE_KEY, M.serialize(ws()));
  } catch {
    // Private browsing or a full store: the workspace lasts until the page closes.
  }
}

/** How a change moves the browser's address: a new history entry, or this one rewritten. */
type UrlChange = "push" | "replace" | "none";

function browserState(w: M.Workspace) {
  const pane = M.activePane(w);
  return { ws: { pane, index: w.panes[pane]?.index ?? 0 } };
}

function syncUrl(w: M.Workspace, how: UrlChange) {
  if (how === "none") return;
  // With no tab open the address is the bare root, which reopens nothing.
  const url = M.isEmpty(w) ? "/" : M.location(w, M.activePane(w));
  const here = window.location.pathname + window.location.search + window.location.hash;
  const state = browserState(w);
  if (how === "push" && url !== here) window.history.pushState(state, "", url);
  else window.history.replaceState(state, "", url);
}

/** Apply `fn` to the workspace, save it, and move the browser's address as `how` says. */
export function update(fn: (w: M.Workspace) => M.Workspace, how: UrlChange = "replace") {
  const before = ws();
  const after = fn(before);
  if (after === before) return;
  setWs(after);
  save();
  syncUrl(after, how);
}

let loaded = false;

/** The browser's router wrote the address: put the active pane's back, if the workspace shows. */
export function resyncUrl() {
  if (loaded) syncUrl(ws(), "replace");
}

/** The workspace stopped showing (a logout): the address is the browser router's again. */
export function unloadWorkspace() {
  saveNow();
  loaded = false;
}

/** Load the stored workspace and open `url` in its active pane. */
export function loadWorkspace(url: string) {
  loaded = true;
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(WORKSPACE_KEY);
  } catch {
    raw = null;
  }
  const restored = M.restore(raw);
  setWs(M.openAtLoad(restored, url));
  save();
  syncUrl(ws(), "replace");
}

/** Replace the whole workspace, as the tests and a reset do. */
export function resetWorkspace(next: M.Workspace) {
  setWs(next);
  save();
  syncUrl(next, "replace");
}

// The actions panes, tabs and links call.

export function navigatePane(pane: string, url: string, replace = false) {
  // No pane to show it in (every tab closed): it gets a tab of its own.
  if (!ws().panes[pane]) {
    openTab(url);
    return;
  }
  const active = M.activePane(ws()) === pane;
  update((w) => M.navigate(w, pane, url, replace), active ? (replace ? "replace" : "push") : "none");
}

export function goPane(pane: string, delta: number) {
  const active = M.activePane(ws()) === pane;
  update((w) => M.go(w, pane, delta), active ? "replace" : "none");
}

export function activatePane(pane: string) {
  update((w) => M.activatePane(w, pane));
}

export function activateTab(id: string) {
  update((w) => M.activateTab(w, id));
}

/** Open `url` in a new tab; a background tab leaves the current one showing. */
export function openTab(url: string, background = false) {
  update((w) => M.openTab(w, url, { background }), background ? "none" : "push");
}

export function closeTab(id: string) {
  update((w) => M.closeTab(w, id));
}

export function closePane(pane: string) {
  update((w) => M.closePane(w, pane));
}

/**
 * Show `url` in a pane beside `pane`: the pane in the same tab that shows it already becomes
 * active, or one whose page `reuse` accepts goes there, else a new pane opens on `pane`'s right. The browser box's desktop opens this way from
 * an agent's conversation (browser B4).
 */
export function openBeside(pane: string, url: string, reuse?: (path: string) => boolean) {
  const w = ws();
  const tab = w.tabs.find((t) => M.panesOf(t.layout).includes(pane)) ?? w.tabs.find((t) => t.id === w.active);
  const path = url.split("?")[0]!;
  const fits = reuse ?? ((p: string) => p === path);
  const shown = tab && M.panesOf(tab.layout).find((p) => p !== pane && fits(M.location(w, p).split("?")[0]!));
  if (shown) {
    // A pane beside that shows another page of the kind goes to this one, as a sub agent's
    // session pane does when another block is clicked (subagents U6).
    if (M.location(w, shown).split("?")[0] !== path) navigatePane(shown, url);
    activatePane(shown);
    return;
  }
  if (!w.panes[pane]) {
    openTab(url);
    return;
  }
  update((x) => M.splitPane(x, pane, "right", url), "push");
}

export function splitPane(pane: string, edge: M.Edge) {
  update((w) => M.splitPane(w, pane, edge));
}

export function dockPane(pane: string, target: string, edge: M.Edge) {
  update((w) => M.dockPane(w, pane, target, edge));
}

export function swapPanes(a: string, b: string) {
  update((w) => M.swapPanes(w, a, b));
}

export function movePaneToTab(pane: string, tab: string) {
  update((w) => M.movePaneToTab(w, pane, tab));
}

export function movePaneToNewTab(pane: string, index?: number) {
  update((w) => M.movePaneToNewTab(w, pane, index));
}

export function moveTab(id: string, to: number) {
  update((w) => M.moveTab(w, id, to), "none");
}

export function setRatio(split: string, ratio: number) {
  update((w) => M.setRatio(w, split, ratio), "none");
}

export function cycleTab(delta: number) {
  update((w) => M.cycleTab(w, delta));
}

/** The browser went back or forward: show the pane and place it named, or go there in the active pane. */
export function onPopState(state: unknown, url: string) {
  const s = (state as { ws?: { pane?: unknown; index?: unknown } } | null)?.ws;
  const w = ws();
  if (s && typeof s.pane === "string" && typeof s.index === "number" && w.panes[s.pane]) {
    const pane = s.pane;
    const index = s.index;
    if (w.panes[pane]!.history[index] === url) {
      update((x) => M.setIndex(M.activatePane(x, pane), pane, index), "none");
      return;
    }
  }
  if (!M.isAppPath(url)) return;
  const pane = M.activePane(w);
  const p = w.panes[pane];
  if (!p) {
    if (url !== "/") update((x) => M.openTab(x, url), "replace");
    return;
  }
  // A step the pane itself remembers is taken there, so its forward history survives.
  if (p.history[p.index - 1] === url) update((x) => M.go(x, pane, -1), "replace");
  else if (p.history[p.index + 1] === url) update((x) => M.go(x, pane, 1), "replace");
  else update((x) => M.navigate(x, pane, url), "replace");
}
