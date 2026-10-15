// The two sidebars around the workspace: the left (navigation) and the right (context for the
// active pane, a column of panels). Each is open or collapsed and has a width; both persist per
// browser. On a phone they are drawers instead, closed until asked for.

import { type JSX, createRoot, createSignal } from "solid-js";
import type { Note } from "./api";

export const SIDEBARS_KEY = "thevault.sidebars";

export const MIN_WIDTH = 180;
export const MAX_WIDTH = 560;
export const LEFT_DEFAULT = 240;
export const RIGHT_DEFAULT = 320;

/** The right sidebar's panels, one shown at a time. */
export const PANELS = ["properties", "history"] as const;
export type Panel = (typeof PANELS)[number];

export type Side = "left" | "right";

export interface SidebarState {
  open: boolean;
  width: number;
}

export interface Sidebars {
  left: SidebarState;
  /** `graph`: whether the graph at the top of the right sidebar is expanded. */
  right: SidebarState & { panel: Panel; graph: boolean };
}

export function defaultSidebars(): Sidebars {
  return { left: { open: true, width: LEFT_DEFAULT }, right: { open: false, width: RIGHT_DEFAULT, panel: "properties", graph: true } };
}

export function clampWidth(w: number): number {
  return Math.round(Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, w)));
}

export function toggleSide(s: Sidebars, side: Side, open = !s[side].open): Sidebars {
  return { ...s, [side]: { ...s[side], open } };
}

export function setWidth(s: Sidebars, side: Side, width: number): Sidebars {
  return { ...s, [side]: { ...s[side], width: clampWidth(width) } };
}

/** Open the right sidebar on `panel`. */
export function showPanel(s: Sidebars, panel: Panel): Sidebars {
  return { ...s, right: { ...s.right, open: true, panel } };
}

/** Sidebars from whatever was stored: anything missing or malformed takes its default. */
export function readSidebars(raw: unknown): Sidebars {
  const d = defaultSidebars();
  let data: unknown = raw;
  if (typeof raw === "string") {
    try {
      data = JSON.parse(raw);
    } catch {
      return d;
    }
  }
  if (typeof data !== "object" || data === null) return d;
  const side = (v: unknown, def: SidebarState): SidebarState => {
    if (typeof v !== "object" || v === null) return def;
    const o = v as Record<string, unknown>;
    return {
      open: typeof o.open === "boolean" ? o.open : def.open,
      width: typeof o.width === "number" && Number.isFinite(o.width) ? clampWidth(o.width) : def.width,
    };
  };
  const o = data as Record<string, unknown>;
  const right = o.right as Record<string, unknown> | undefined;
  const panel = PANELS.find((p) => p === right?.panel) ?? d.right.panel;
  const graph = typeof right?.graph === "boolean" ? right.graph : d.right.graph;
  return { left: side(o.left, d.left), right: { ...side(o.right, d.right), panel, graph } };
}

function load(): Sidebars {
  try {
    return readSidebars(localStorage.getItem(SIDEBARS_KEY));
  } catch {
    return defaultSidebars();
  }
}

const state = createRoot(() => {
  const [sidebars, set] = createSignal<Sidebars>(load());
  // Phone drawers: never stored, closed on every load.
  const [leftDrawer, setLeftDrawer] = createSignal(false);
  const [rightDrawer, setRightDrawer] = createSignal(false);
  return { sidebars, set, leftDrawer, setLeftDrawer, rightDrawer, setRightDrawer };
});

export const sidebars = state.sidebars;
export const leftDrawer = state.leftDrawer;
export const rightDrawer = state.rightDrawer;
export const setLeftDrawer = state.setLeftDrawer;
export const setRightDrawer = state.setRightDrawer;

function update(fn: (s: Sidebars) => Sidebars) {
  const next = fn(state.sidebars());
  state.set(next);
  try {
    localStorage.setItem(SIDEBARS_KEY, JSON.stringify(next));
  } catch {
    // Private browsing: the layout lasts until the page closes.
  }
}

/** The left sidebar is a drawer at this width and below, the right at the phone width. */
const LEFT_DRAWER = "(max-width: 1024px)";
const RIGHT_DRAWER = "(max-width: 600px)";

function matches(q: string): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia(q).matches;
}

export function rightIsDrawer(): boolean {
  return matches(RIGHT_DRAWER);
}

/** Show or hide a sidebar: its drawer on a narrow screen, else its column. */
export function toggleSidebar(side: Side, open?: boolean) {
  if (side === "left" && matches(LEFT_DRAWER)) {
    setLeftDrawer(open ?? !leftDrawer());
    return;
  }
  if (side === "right" && matches(RIGHT_DRAWER)) {
    setRightDrawer(open ?? !rightDrawer());
    return;
  }
  update((s) => toggleSide(s, side, open));
}

/** Whether a sidebar shows now, drawer or column. Reactive. */
export function sidebarShown(side: Side, drawer: boolean): boolean {
  if (drawer) return side === "left" ? leftDrawer() : rightDrawer();
  return sidebars()[side].open;
}

export function resizeSidebar(side: Side, width: number) {
  update((s) => setWidth(s, side, width));
}

/** Show `panel` in the right sidebar, open or not. */
export function setPanel(panel: Panel) {
  if (state.sidebars().right.panel !== panel) update((s) => ({ ...s, right: { ...s.right, panel } }));
}

/** Expand or collapse the graph at the top of the right sidebar. */
export function setGraphOpen(graph: boolean) {
  if (state.sidebars().right.graph !== graph) update((s) => ({ ...s, right: { ...s.right, graph } }));
}

/** Open the right sidebar on `panel` (its drawer on a phone) and move focus to it. */
export function focusPanel(panel: Panel) {
  if (matches(RIGHT_DRAWER)) {
    setPanel(panel);
    setRightDrawer(true);
  } else update((s) => showPanel(s, panel));
  queueMicrotask(() => document.getElementById(`${panel}-panel`)?.focus());
}

/** Reactive: whether the right sidebar shows `panel` now, as a column or a drawer. */
export function panelShown(panel: Panel): boolean {
  return sidebarShown("right", rightIsDrawer()) && sidebars().right.panel === panel;
}

/**
 * What a mounted note page gives the right sidebar's panels: the editors of its unplaced fields
 * (Properties), and its note's version and save path (History).
 */
export interface NoteSource {
  /** The note's id; empty for a new note not created yet. */
  noteId: () => string;
  /** Reactive: the stored version the page holds, which changes on every save. */
  version: () => string;
  render: () => JSX.Element;
  /** Save any unsaved edits now. False when they could not be saved (a conflict, an error). */
  flush: () => Promise<boolean>;
  /** Take `note`, just written by a restore, as the live note. */
  restored: (note: Note) => void;
}

const [sources, setSources] = createRoot(() => createSignal<Record<string, NoteSource>>({}));

/** A note page in `pane` offers itself to the panels; returns the withdrawal. */
export function offerNote(pane: string, source: NoteSource): () => void {
  setSources((all) => ({ ...all, [pane]: source }));
  return () =>
    setSources((all) => {
      if (all[pane] !== source) return all;
      const next = { ...all };
      delete next[pane];
      return next;
    });
}

/** Reactive: what the note page in `pane` offers, if it shows one. */
export function noteOf(pane: string): NoteSource | undefined {
  return sources()[pane];
}
