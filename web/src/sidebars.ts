// The two sidebars around the workspace: the left (navigation) and the right (context for the
// active pane, a column of panels). Each is open or collapsed and has a width; both persist per
// browser. On a phone they are drawers instead, closed until asked for.

import { type JSX, createRoot, createSignal } from "solid-js";

export const SIDEBARS_KEY = "thevault.sidebars";

export const MIN_WIDTH = 180;
export const MAX_WIDTH = 560;
export const LEFT_DEFAULT = 240;
export const RIGHT_DEFAULT = 320;

/** The right sidebar's panels. Only one so far. */
export const PANELS = ["properties"] as const;
export type Panel = (typeof PANELS)[number];

export type Side = "left" | "right";

export interface SidebarState {
  open: boolean;
  width: number;
}

export interface Sidebars {
  left: SidebarState;
  right: SidebarState & { panel: Panel };
}

export function defaultSidebars(): Sidebars {
  return { left: { open: true, width: LEFT_DEFAULT }, right: { open: false, width: RIGHT_DEFAULT, panel: "properties" } };
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
  return { left: side(o.left, d.left), right: { ...side(o.right, d.right), panel } };
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

/** Open the right sidebar on `panel` and move focus to it. */
export function focusPanel(panel: Panel) {
  if (matches(RIGHT_DRAWER)) setRightDrawer(true);
  else update((s) => showPanel(s, panel));
  queueMicrotask(() => document.getElementById(`${panel}-panel`)?.focus());
}

/** What a mounted note page gives the Properties panel: the editors of its unplaced fields. */
export interface PropertiesSource {
  noteId: () => string;
  render: () => JSX.Element;
}

const [sources, setSources] = createRoot(() => createSignal<Record<string, PropertiesSource>>({}));

/** A note page in `pane` offers its properties; returns the withdrawal. */
export function offerProperties(pane: string, source: PropertiesSource): () => void {
  setSources((all) => ({ ...all, [pane]: source }));
  return () =>
    setSources((all) => {
      if (all[pane] !== source) return all;
      const next = { ...all };
      delete next[pane];
      return next;
    });
}

/** Reactive: the properties the note page in `pane` offers, if it shows one. */
export function propertiesOf(pane: string): PropertiesSource | undefined {
  return sources()[pane];
}
