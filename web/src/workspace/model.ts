// The workspace: tabs, each holding a layout tree of panes, and every pane's own history. Every
// function here is pure: it takes a workspace and returns a new one, leaving the old untouched,
// so the store can keep the last value and tests can compare before and after.

/** Where a fresh pane or tab goes when nothing else is asked for. */
export const HOME = "/daily";

/** How many locations a pane remembers behind and ahead of the one it shows. */
export const MAX_HISTORY = 50;

/** Splits never make a side thinner than this share of their space. */
export const MIN_RATIO = 0.1;

/** `row` puts its children side by side, `column` stacks them. */
export type SplitDir = "row" | "column";

export type Edge = "left" | "right" | "top" | "bottom";

/** A leaf is one pane; a split shares its space between `a` (left or top) and `b`. */
export type Layout = { pane: string } | { split: string; dir: SplitDir; ratio: number; a: Layout; b: Layout };

export interface PaneState {
  /** The locations visited, oldest first; `index` is the one shown. */
  history: string[];
  index: number;
}

export interface TabState {
  id: string;
  layout: Layout;
  /** The pane of this tab that was last active. */
  active: string;
}

export interface Workspace {
  tabs: TabState[];
  /** The id of the tab shown. */
  active: string;
  panes: Record<string, PaneState>;
}

let counter = 0;

/** A fresh id, unique within this page and very likely across reloads. */
export function newId(prefix: string): string {
  counter += 1;
  return `${prefix}${Date.now().toString(36)}${counter.toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

export function isLeaf(l: Layout): l is { pane: string } {
  return "pane" in l;
}

/** The panes of a layout, left to right and top to bottom. */
export function panesOf(l: Layout): string[] {
  return isLeaf(l) ? [l.pane] : [...panesOf(l.a), ...panesOf(l.b)];
}

/** `layout` without the leaf for `pane`; a split left with one side becomes that side. */
export function removeLeaf(l: Layout, pane: string): Layout | null {
  if (isLeaf(l)) return l.pane === pane ? null : l;
  const a = removeLeaf(l.a, pane);
  const b = removeLeaf(l.b, pane);
  if (!a) return b;
  if (!b) return a;
  return a === l.a && b === l.b ? l : { ...l, a, b };
}

/** `layout` with the leaf for `pane` replaced by what `fn` makes of it. */
export function replaceLeaf(l: Layout, pane: string, fn: (leaf: { pane: string }) => Layout): Layout {
  if (isLeaf(l)) return l.pane === pane ? fn(l) : l;
  const a = replaceLeaf(l.a, pane, fn);
  const b = replaceLeaf(l.b, pane, fn);
  return a === l.a && b === l.b ? l : { ...l, a, b };
}

/** A split putting `moved` on the `edge` side of `target`. */
function beside(target: Layout, moved: Layout, edge: Edge): Layout {
  const dir: SplitDir = edge === "left" || edge === "right" ? "row" : "column";
  const first = edge === "left" || edge === "top";
  return { split: newId("s"), dir, ratio: 0.5, a: first ? moved : target, b: first ? target : moved };
}

/** The tab showing. With no tabs open, a stand-in with no pane, so callers need no special case. */
export function activeTab(ws: Workspace): TabState {
  return ws.tabs.find((t) => t.id === ws.active) ?? ws.tabs[0] ?? { id: "", layout: { pane: "" }, active: "" };
}

/** The active pane, or "" when no tab is open. */
export function activePane(ws: Workspace): string {
  return activeTab(ws).active;
}

export function tabOf(ws: Workspace, pane: string): TabState | undefined {
  return ws.tabs.find((t) => panesOf(t.layout).includes(pane));
}

/** The location `pane` shows. */
export function location(ws: Workspace, pane: string): string {
  const p = ws.panes[pane];
  return p?.history[p.index] ?? HOME;
}

export function canGo(ws: Workspace, pane: string, delta: number): boolean {
  const p = ws.panes[pane];
  if (!p) return false;
  const i = p.index + delta;
  return i >= 0 && i < p.history.length;
}

function freshPane(url: string): PaneState {
  return { history: [url], index: 0 };
}

/** A workspace with no tabs open: the main area is empty until something opens one. */
export function noTabs(): Workspace {
  return { tabs: [], active: "", panes: {} };
}

/** Whether no tab is open. */
export function isEmpty(ws: Workspace): boolean {
  return ws.tabs.length === 0;
}

/** A workspace with one tab of one pane showing `url`. */
export function emptyWorkspace(url = HOME): Workspace {
  const pane = newId("p");
  const tab = newId("t");
  return { tabs: [{ id: tab, layout: { pane }, active: pane }], active: tab, panes: { [pane]: freshPane(url) } };
}

function withTab(ws: Workspace, id: string, fn: (t: TabState) => TabState): Workspace {
  return { ...ws, tabs: ws.tabs.map((t) => (t.id === id ? fn(t) : t)) };
}

/** Open `url` in a new tab of one pane, at the end, shown unless `background`. */
export function openTab(ws: Workspace, url: string, opts: { background?: boolean } = {}): Workspace {
  const pane = newId("p");
  const tab: TabState = { id: newId("t"), layout: { pane }, active: pane };
  return {
    tabs: [...ws.tabs, tab],
    active: opts.background ? ws.active : tab.id,
    panes: { ...ws.panes, [pane]: freshPane(url) },
  };
}

/** The tab to show when `index` closes: the one after it, else the one before. */
function neighbour(tabs: TabState[], index: number): string | undefined {
  return (tabs[index + 1] ?? tabs[index - 1])?.id;
}

/** Close tab `id` and its panes. Closing the last tab leaves no tabs open. */
export function closeTab(ws: Workspace, id: string): Workspace {
  const index = ws.tabs.findIndex((t) => t.id === id);
  if (index < 0) return ws;
  const tab = ws.tabs[index]!;
  const panes = { ...ws.panes };
  for (const p of panesOf(tab.layout)) delete panes[p];
  const tabs = ws.tabs.filter((t) => t.id !== id);
  if (!tabs.length) return noTabs();
  const active = ws.active === id ? neighbour(ws.tabs, index)! : ws.active;
  return { tabs, active, panes };
}

/** Move tab `id` to `to`, an index among the tabs as they stand without it. */
export function moveTab(ws: Workspace, id: string, to: number): Workspace {
  const tab = ws.tabs.find((t) => t.id === id);
  if (!tab) return ws;
  const rest = ws.tabs.filter((t) => t.id !== id);
  const at = Math.max(0, Math.min(to, rest.length));
  rest.splice(at, 0, tab);
  return rest.every((t, i) => t === ws.tabs[i]) ? ws : { ...ws, tabs: rest };
}

export function activateTab(ws: Workspace, id: string): Workspace {
  return ws.active === id || !ws.tabs.some((t) => t.id === id) ? ws : { ...ws, active: id };
}

/** Show `pane`'s tab and make `pane` its active pane. */
export function activatePane(ws: Workspace, pane: string): Workspace {
  const tab = tabOf(ws, pane);
  if (!tab || (ws.active === tab.id && tab.active === pane)) return ws;
  return { ...withTab(ws, tab.id, (t) => ({ ...t, active: pane })), active: tab.id };
}

/** Switch to the tab `delta` places along, wrapping round. */
export function cycleTab(ws: Workspace, delta: number): Workspace {
  const n = ws.tabs.length;
  if (!n) return ws;
  const i = ws.tabs.findIndex((t) => t.id === ws.active);
  return activateTab(ws, ws.tabs[(((i + delta) % n) + n) % n]!.id);
}

/**
 * Split `pane`, putting a new pane on its `edge` side showing `url` (by default what `pane`
 * shows). The new pane becomes active.
 */
export function splitPane(ws: Workspace, pane: string, edge: Edge = "right", url?: string): Workspace {
  const tab = tabOf(ws, pane);
  if (!tab) return ws;
  const id = newId("p");
  const layout = replaceLeaf(tab.layout, pane, (leaf) => beside(leaf, { pane: id }, edge));
  const next = withTab(ws, tab.id, (t) => ({ ...t, layout, active: id }));
  return { ...next, active: tab.id, panes: { ...ws.panes, [id]: freshPane(url ?? location(ws, pane)) } };
}

/**
 * Take `pane` out of its tab, keeping its state. A tab left with no panes goes; the tab shown
 * then is its neighbour. Used by every move.
 */
function detach(ws: Workspace, pane: string): Workspace {
  const index = ws.tabs.findIndex((t) => panesOf(t.layout).includes(pane));
  if (index < 0) return ws;
  const tab = ws.tabs[index]!;
  const layout = removeLeaf(tab.layout, pane);
  if (!layout) {
    const tabs = ws.tabs.filter((t) => t.id !== tab.id);
    return { ...ws, tabs, active: ws.active === tab.id ? (neighbour(ws.tabs, index) ?? ws.active) : ws.active };
  }
  return withTab(ws, tab.id, (t) => ({ ...t, layout, active: t.active === pane ? siblingOf(tab.layout, pane, layout) : t.active }));
}

/** The pane that takes over when `pane` leaves `before`: the nearest pane of its sibling. */
function siblingOf(before: Layout, pane: string, after: Layout): string {
  const walk = (l: Layout): string | null => {
    if (isLeaf(l)) return null;
    if (isLeaf(l.a) && l.a.pane === pane) return panesOf(l.b)[0]!;
    if (isLeaf(l.b) && l.b.pane === pane) return panesOf(l.a).at(-1)!;
    return walk(l.a) ?? walk(l.b);
  };
  return walk(before) ?? panesOf(after)[0]!;
}

/** Close `pane`; its split collapses. Closing a tab's last pane closes the tab. */
export function closePane(ws: Workspace, pane: string): Workspace {
  const tab = tabOf(ws, pane);
  if (!tab) return ws;
  if (panesOf(tab.layout).length === 1) return closeTab(ws, tab.id);
  const next = detach(ws, pane);
  const panes = { ...next.panes };
  delete panes[pane];
  return { ...next, panes };
}

/** Move `pane` to the `edge` side of `target`, in whichever tab `target` is. */
export function dockPane(ws: Workspace, pane: string, target: string, edge: Edge): Workspace {
  if (pane === target || !tabOf(ws, pane) || !tabOf(ws, target)) return ws;
  const out = detach(ws, pane);
  const tab = tabOf(out, target)!;
  const layout = replaceLeaf(tab.layout, target, (leaf) => beside(leaf, { pane }, edge));
  return { ...withTab(out, tab.id, (t) => ({ ...t, layout, active: pane })), active: tab.id };
}

/** Swap the places of `a` and `b`, which may be in different tabs. `a` becomes active. */
export function swapPanes(ws: Workspace, a: string, b: string): Workspace {
  const ta = tabOf(ws, a);
  const tb = tabOf(ws, b);
  if (a === b || !ta || !tb) return ws;
  const swap = (l: Layout) =>
    replaceLeaf(replaceLeaf(l, a, () => ({ pane: "\u0000" })), b, () => ({ pane: a }));
  const fill = (l: Layout) => replaceLeaf(l, "\u0000", () => ({ pane: b }));
  const tabs = ws.tabs.map((t) => {
    if (t.id !== ta.id && t.id !== tb.id) return t;
    const layout = fill(swap(t.layout));
    const active = t.id === tb.id ? a : t.active === a ? b : t.active;
    return { ...t, layout, active };
  });
  return { ...ws, tabs, active: tb.id };
}

/** Move `pane` into tab `id`, to the right of that tab's active pane, and show it there. */
export function movePaneToTab(ws: Workspace, pane: string, id: string): Workspace {
  const from = tabOf(ws, pane);
  const to = ws.tabs.find((t) => t.id === id);
  if (!from || !to || from.id === id) return ws;
  return dockPane(ws, pane, to.active, "right");
}

/** Move `pane` into a tab of its own, placed at `index` (by default the end), and show it. */
export function movePaneToNewTab(ws: Workspace, pane: string, index?: number): Workspace {
  const from = tabOf(ws, pane);
  if (!from) return ws;
  if (panesOf(from.layout).length === 1) {
    const moved = index === undefined ? ws : moveTab(ws, from.id, index);
    return activateTab(moved, from.id);
  }
  const out = detach(ws, pane);
  const tab: TabState = { id: newId("t"), layout: { pane }, active: pane };
  const tabs = [...out.tabs];
  tabs.splice(index === undefined ? tabs.length : Math.max(0, Math.min(index, tabs.length)), 0, tab);
  return { ...out, tabs, active: tab.id };
}

function clampRatio(r: number): number {
  return Math.max(MIN_RATIO, Math.min(1 - MIN_RATIO, r));
}

/** Set the share split `id` gives its first side. */
export function setRatio(ws: Workspace, id: string, ratio: number): Workspace {
  const r = clampRatio(ratio);
  const walk = (l: Layout): Layout => {
    if (isLeaf(l)) return l;
    if (l.split === id) return l.ratio === r ? l : { ...l, ratio: r };
    const a = walk(l.a);
    const b = walk(l.b);
    return a === l.a && b === l.b ? l : { ...l, a, b };
  };
  const tabs = ws.tabs.map((t) => {
    const layout = walk(t.layout);
    return layout === t.layout ? t : { ...t, layout };
  });
  return tabs.every((t, i) => t === ws.tabs[i]) ? ws : { ...ws, tabs };
}

/** The ratio of split `id`, if there is one. */
export function ratioOf(ws: Workspace, id: string): number | undefined {
  const walk = (l: Layout): number | undefined =>
    isLeaf(l) ? undefined : l.split === id ? l.ratio : (walk(l.a) ?? walk(l.b));
  for (const t of ws.tabs) {
    const r = walk(t.layout);
    if (r !== undefined) return r;
  }
  return undefined;
}

/**
 * Show `url` in `pane`. A push drops the locations ahead and adds one; a replace changes the one
 * shown. Going to where the pane already is changes nothing.
 */
export function navigate(ws: Workspace, pane: string, url: string, replace = false): Workspace {
  const p = ws.panes[pane];
  if (!p) return ws;
  if (p.history[p.index] === url) return ws;
  let next: PaneState;
  if (replace) {
    const history = [...p.history];
    history[p.index] = url;
    next = { history, index: p.index };
  } else {
    const history = [...p.history.slice(0, p.index + 1), url].slice(-MAX_HISTORY);
    next = { history, index: history.length - 1 };
  }
  return { ...ws, panes: { ...ws.panes, [pane]: next } };
}

/** Move `pane` `delta` steps through its history, stopping at either end. */
export function go(ws: Workspace, pane: string, delta: number): Workspace {
  const p = ws.panes[pane];
  if (!p) return ws;
  return setIndex(ws, pane, p.index + delta);
}

/** Show the location at `index` in `pane`'s history. */
export function setIndex(ws: Workspace, pane: string, index: number): Workspace {
  const p = ws.panes[pane];
  if (!p) return ws;
  const i = Math.max(0, Math.min(index, p.history.length - 1));
  return i === p.index ? ws : { ...ws, panes: { ...ws.panes, [pane]: { ...p, index: i } } };
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Divider {
  split: string;
  dir: SplitDir;
  ratio: number;
  /** The space the split shares, as fractions of the tab's area. */
  area: Rect;
}

/** Where each pane and divider of `layout` sits, as fractions of the tab's area. */
export function layoutRects(layout: Layout): { panes: (Rect & { pane: string })[]; dividers: Divider[] } {
  const panes: (Rect & { pane: string })[] = [];
  const dividers: Divider[] = [];
  const walk = (l: Layout, r: Rect) => {
    if (isLeaf(l)) {
      panes.push({ pane: l.pane, ...r });
      return;
    }
    dividers.push({ split: l.split, dir: l.dir, ratio: l.ratio, area: r });
    if (l.dir === "row") {
      walk(l.a, { ...r, w: r.w * l.ratio });
      walk(l.b, { ...r, x: r.x + r.w * l.ratio, w: r.w * (1 - l.ratio) });
    } else {
      walk(l.a, { ...r, h: r.h * l.ratio });
      walk(l.b, { ...r, y: r.y + r.h * l.ratio, h: r.h * (1 - l.ratio) });
    }
  };
  walk(layout, { x: 0, y: 0, w: 1, h: 1 });
  return { panes, dividers };
}

/** Which part of a pane a point at fractions `fx`, `fy` of it falls in, for docking. */
export function dropZone(fx: number, fy: number): Edge | "center" {
  const edges: [Edge, number][] = [
    ["left", fx],
    ["right", 1 - fx],
    ["top", fy],
    ["bottom", 1 - fy],
  ];
  const [edge, d] = edges.reduce((best, e) => (e[1] < best[1] ? e : best));
  return d < 0.25 ? edge : "center";
}

/** Only a path inside the app; never the login page, never another origin. */
export function isAppPath(url: unknown): url is string {
  return typeof url === "string" && url.startsWith("/") && !url.startsWith("//") && !url.startsWith("/login") && !url.startsWith("/api/");
}

/**
 * The workspace after the page loads at `url`: the active pane goes there, unless it is there
 * already, so reloading leaves its history as it was.
 */
export function openAtLoad(ws: Workspace, url: string): Workspace {
  if (!isAppPath(url) || url === "/") return ws;
  if (isEmpty(ws)) return openTab(ws, url);
  return navigate(ws, activePane(ws), url);
}

const STORE_VERSION = 1;

export function serialize(ws: Workspace): string {
  return JSON.stringify({ version: STORE_VERSION, ...ws });
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function readPane(v: unknown): PaneState | null {
  if (!isObject(v) || !Array.isArray(v.history)) return null;
  const history = v.history.filter(isAppPath).slice(-MAX_HISTORY);
  if (!history.length) return null;
  const index = typeof v.index === "number" && Number.isInteger(v.index) ? Math.max(0, Math.min(v.index, history.length - 1)) : history.length - 1;
  return { history, index };
}

/**
 * A workspace from what was stored, whatever it holds: anything malformed is dropped, a split
 * that loses a side becomes the other side, and nothing left means a fresh workspace.
 */
export function restore(raw: unknown): Workspace {
  let data: unknown = raw;
  if (typeof raw === "string") {
    try {
      data = JSON.parse(raw);
    } catch {
      return emptyWorkspace();
    }
  }
  if (!isObject(data) || data.version !== STORE_VERSION || !Array.isArray(data.tabs) || !isObject(data.panes)) {
    return emptyWorkspace();
  }
  const stored = data.panes;
  const panes: Record<string, PaneState> = {};
  const used = new Set<string>();
  const readLayout = (v: unknown, depth: number): Layout | null => {
    if (!isObject(v) || depth > 32) return null;
    if (typeof v.pane === "string") {
      if (used.has(v.pane)) return null;
      const p = readPane(stored[v.pane]);
      if (!p) return null;
      used.add(v.pane);
      panes[v.pane] = p;
      return { pane: v.pane };
    }
    if (typeof v.split !== "string" || (v.dir !== "row" && v.dir !== "column")) return null;
    const a = readLayout(v.a, depth + 1);
    const b = readLayout(v.b, depth + 1);
    if (!a || !b) return a ?? b;
    const ratio = typeof v.ratio === "number" && Number.isFinite(v.ratio) ? clampRatio(v.ratio) : 0.5;
    return { split: v.split, dir: v.dir, ratio, a, b };
  };
  const tabs: TabState[] = [];
  const tabIds = new Set<string>();
  for (const t of data.tabs) {
    if (!isObject(t) || typeof t.id !== "string" || tabIds.has(t.id)) continue;
    const layout = readLayout(t.layout, 0);
    if (!layout) continue;
    const inTab = panesOf(layout);
    const active = typeof t.active === "string" && inTab.includes(t.active) ? t.active : inTab[0]!;
    tabIds.add(t.id);
    tabs.push({ id: t.id, layout, active });
  }
  // Every tab closed on purpose stays closed; tabs that were all unreadable give a fresh start.
  if (!tabs.length) return data.tabs.length === 0 ? noTabs() : emptyWorkspace();
  const active = typeof data.active === "string" && tabIds.has(data.active) ? data.active : tabs[0]!.id;
  return { tabs, active, panes };
}
