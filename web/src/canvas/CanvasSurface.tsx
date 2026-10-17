// The one canvas component (D15): a canvas content block and a canvas field both render through
// it. The view pans by dragging the background and zooms with the wheel, a pinch, or the zoom
// controls behind the magnifying glass in its corner. While editing, "+" in the corner adds
// cards, notes and groups; nodes move by dragging and resize from their corner, and an edge is
// drawn by dragging from a port on a node's side to another node. A card holds content blocks:
// a double-click, or Enter on the selected card, edits them in place with the block editor, and
// Escape or a click outside the card stops. A selected node, group or edge shows a small cluster
// of buttons: its colour, delete, Open for a note, and an edge's arrows and caption. A group's
// label is renamed by double-clicking it. The handle on the bottom edge sets the canvas's height.
// Edges run in right-angled elbows between the closest pair of ports, and the saved sides follow
// every move. Every change, the view included, reports the whole new block. Without `onChange`
// the canvas is shown, and the view moves without being saved. `body` draws a node in place of
// its kind's own body, which is how a canvas query view shows its results as cards.

import { type JSX, For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount, untrack } from "solid-js";
import type { Editor } from "@tiptap/core";
import type { Block, Json } from "../api";
import { api } from "../api";
import { HeightHandle } from "../components/HeightHandle";
import { ContentEditor } from "../editor/ContentEditor";
import { openInBackground, wantsNewTab } from "../workspace/events";
import { openTab } from "../workspace/store";
import {
  type Arrow,
  type Canvas,
  type CanvasEdge,
  type CanvasNode,
  type NodeKind,
  type Point,
  type Side,
  type Viewport,
  ARROWS,
  CANVAS_HEIGHT,
  NEW_NODE,
  SIDES,
  addEdge,
  addGroup,
  addNode,
  arrowEnds,
  arrowHead,
  arrowItem,
  canvasOf,
  clampHeight,
  closestSides,
  contains,
  edgeRoute,
  elbowRoute,
  fitView,
  routeDirection,
  midpoint,
  moveItem,
  nearestSide,
  portPoint,
  removeItem,
  resizeItem,
  resnapEdges,
  roundedPath,
  setItem,
  setNodeBlocks,
  toCanvas,
  trimRoute,
  withHeight,
  withViewport,
  zoomAbout,
} from "./canvas";
import { ArrowIcon, CaptionIcon, MagnifierIcon, OpenIcon, TrashIcon } from "./CanvasIcons";
import { NodeBody } from "./CanvasNodes";
import { PALETTE_COLOURS, colourItem, colourLabel, paletteCss } from "./palette";

/** Zoom steps for the buttons, and for one notch of the wheel. */
const ZOOM_STEP = 1.25;
const WHEEL_STEP = 1.1;

/** Wait this long after the last wheel notch before saving the view. */
const WHEEL_SETTLE_MS = 250;

/** A pointer that moves less than this, in screen pixels, clicked rather than dragged. */
const CLICK_SLOP = 3;

/** Two presses this close in time and place are a double-click. */
const DOUBLE_MS = 450;
const DOUBLE_SLOP = 6;

/** The `/` menu items a card leaves out: a card cannot hold a canvas. */
const CARD_EXCLUDES = ["canvas"];

/** Popups the card editor opens outside the card, where a press does not leave the card. */
const EDITOR_POPUPS = ".suggest-menu, .editor-bubble, .mention-picker";

/** Elements inside a node that take their own presses rather than moving it. */
const INTERACTIVE = "a, button, input, textarea, select, [contenteditable='true'], summary, [role='tab'], [role='option'], [role='link'], [data-node-view-interactive]";

const ARROW_LABELS: Record<Arrow, string> = {
  none: "No arrows",
  start: "Arrow at start",
  end: "Arrow at end",
  both: "Arrows at both ends",
};

type Drag =
  | { kind: "pan"; pointer: number; sx: number; sy: number; from: Viewport; moved: boolean }
  | { kind: "move"; pointer: number; sx: number; sy: number; id: string; moved: boolean }
  | { kind: "resize"; pointer: number; sx: number; sy: number; id: string; width: number; height: number }
  | { kind: "link"; pointer: number; from: string; side: Side };

/** The note search the "+" popup opens before adding a note node. */
type Pending = { kind: "note"; q: string; results: { id: string; title: string }[] };

let surfaces = 0;

/** This browser session remembers whether the zoom controls are out. */
const ZOOM_KEY = "thevault.canvas-zoom-controls";

function zoomControlsOpen(): boolean {
  try {
    return sessionStorage.getItem(ZOOM_KEY) === "open";
  } catch {
    return false;
  }
}

/** A route as `data-points`: "x,y x,y …", for tests and tools to read. */
const pointsAttr = (pts: Point[]) => pts.map((p) => `${Math.round(p.x * 100) / 100},${Math.round(p.y * 100) / 100}`).join(" ");


export function CanvasSurface(props: {
  block: Block | undefined;
  label: string;
  onChange?: (block: Block) => void;
  body?: (node: CanvasNode) => JSX.Element;
}) {
  const uid = `canvas-${++surfaces}`;
  const stored = createMemo(() => canvasOf(props.block));
  const [view, setView] = createSignal<Viewport>(stored().viewport);
  // A move or resize under way, drawn before it is committed.
  const [preview, setPreview] = createSignal<Block | null>(null);
  const [selected, setSelected] = createSignal<string | null>(null);
  // The nodes whose ports show because the pointer is, or just was, over them.
  const [hovered, setHovered] = createSignal<string[]>([]);
  // An edge being drawn from a port: where it starts and where the pointer is.
  const [linking, setLinking] = createSignal<{ from: string; side: Side; at: Point } | null>(null);
  const [adding, setAdding] = createSignal(false);
  const [zoomOpen, setZoomOpen] = createSignal(zoomControlsOpen());
  const [pending, setPending] = createSignal<Pending | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  // The card whose blocks are being edited, the item whose palette is open, the group being
  // renamed and the edge whose caption is being written.
  const [editingCard, setEditingCard] = createSignal<string | null>(null);
  const [paletteOpen, setPaletteOpen] = createSignal<string | null>(null);
  const [renaming, setRenaming] = createSignal<string | null>(null);
  const [captioning, setCaptioning] = createSignal<string | null>(null);
  // What has been typed into the open caption or group label input. A save's reply replaces the
  // canvas's items while the input is open, which re-renders it (the caption's) or makes it anew
  // (the group label's); the input starts from this rather than the stored text, so nothing typed
  // is lost. Opening or closing either input forgets it.
  let typed: string | null = null;
  createEffect(
    on([captioning, renaming], () => {
      typed = null;
    }),
  );
  /** Fill a caption or label input as it is made: what was typed into it so far, else `stored`. */
  function startTyping(el: HTMLInputElement, stored: string | undefined) {
    const first = typed === null;
    el.value = typed ?? stored ?? "";
    queueMicrotask(() => {
      el.focus({ preventScroll: true });
      if (first) el.select();
    });
  }
  // The height while the bottom handle is dragged.
  const [heightPreview, setHeightPreview] = createSignal<number | null>(null);
  // The view's size on screen, for placing the clusters.
  const [size, setSize] = createSignal({ width: 0, height: 0 });
  let box!: HTMLDivElement;
  let drag: Drag | null = null;
  let wheelTimer: ReturnType<typeof setTimeout> | undefined;
  let lastPress: { key: string; t: number; x: number; y: number } | null = null;
  // Where a double-click asked the card editor to put its caret.
  let caretAt: { x: number; y: number } | null = null;
  const hoverTimers = new Map<string, ReturnType<typeof setTimeout>>();

  const editing = () => !!props.onChange;
  const shown = createMemo<Canvas>(() => {
    const p = preview();
    return p ? canvasOf(p) : stored();
  });
  const byId = createMemo(() => new Map(shown().nodes.map((n) => [n.id, n])));
  const sameIds = (a: string[], b: string[]) => a.length === b.length && a.every((x, i) => x === b[i]);
  const nodeIds = createMemo(() => shown().nodes.map((n) => n.id), undefined, { equals: sameIds });
  // Groups render by id, as nodes do, so a save's reply, which brings new objects for the same
  // groups, updates their frames in place rather than making them anew under an open label input.
  const groupById = createMemo(() => new Map(shown().groups.map((g) => [g.id, g])));
  const groupIds = createMemo(() => shown().groups.map((g) => g.id), undefined, { equals: sameIds });
  const height = () => heightPreview() ?? stored().height ?? CANVAS_HEIGHT.initial;

  // A stored view that changes from outside, such as after a reload, moves the view with it.
  createEffect(
    on(
      () => JSON.stringify(stored().viewport),
      () => setView(stored().viewport),
      { defer: true },
    ),
  );

  function commit(next: Block) {
    setPreview(null);
    props.onChange?.(next);
  }

  /** The block with the current view, which is saved along with every edit. */
  const current = () => withViewport(props.block, view());

  function saveView() {
    if (editing()) commit(current());
  }

  function zoomBy(factor: number, sx?: number, sy?: number) {
    const r = box.getBoundingClientRect();
    setView(zoomAbout(view(), factor, sx ?? r.width / 2, sy ?? r.height / 2));
  }

  function toggleZoom() {
    const next = !zoomOpen();
    setZoomOpen(next);
    try {
      sessionStorage.setItem(ZOOM_KEY, next ? "open" : "closed");
    } catch {
      // Without session storage the controls just forget.
    }
  }

  function fit() {
    const r = box.getBoundingClientRect();
    setView(fitView(shown(), r.width, r.height));
    saveView();
  }

  /** 100%, keeping the canvas point in the middle of the view where it is. */
  function actualSize() {
    const r = box.getBoundingClientRect();
    setView(zoomAbout(view(), 1 / view().zoom, r.width / 2, r.height / 2));
    saveView();
  }

  /**
   * Whether a press is on something inside the canvas that takes its own presses. Only elements
   * inside the view count: a canvas in the block editor sits inside the editor's own editable root.
   */
  const isInteractive = (t: EventTarget | null) => {
    const hit = t instanceof Element ? t.closest(INTERACTIVE) : null;
    return !!hit && hit !== box && box.contains(hit);
  };

  const nodeEl = (id: string) => box?.querySelector<HTMLElement>(`[data-testid="canvas-node"][data-id="${CSS.escape(id)}"]`) ?? null;

  onMount(() => {
    // Not a Solid handler: the wheel must be able to cancel the page's own scroll.
    const onWheel = (e: WheelEvent) => {
      // A card being edited scrolls its own blocks.
      if (e.target instanceof Element && e.target.closest(".canvas-node.editing-card")) return;
      if (!e.ctrlKey && !e.metaKey && !e.altKey && Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
      e.preventDefault();
      const r = box.getBoundingClientRect();
      zoomBy(e.deltaY < 0 ? WHEEL_STEP : 1 / WHEEL_STEP, e.clientX - r.left, e.clientY - r.top);
      clearTimeout(wheelTimer);
      wheelTimer = setTimeout(saveView, WHEEL_SETTLE_MS);
    };
    box.addEventListener("wheel", onWheel, { passive: false });
    const sizer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => setSize({ width: box.clientWidth, height: box.clientHeight }));
    sizer?.observe(box);
    setSize({ width: box.clientWidth, height: box.clientHeight });
    // A press anywhere outside the card being edited stops editing it.
    const onDocDown = (e: PointerEvent) => {
      const id = editingCard();
      if (!id) return;
      const t = e.target;
      if (t instanceof Element && (nodeEl(id)?.contains(t) || t.closest(EDITOR_POPUPS))) return;
      stopEditing();
    };
    document.addEventListener("pointerdown", onDocDown, true);
    onCleanup(() => {
      box.removeEventListener("wheel", onWheel);
      document.removeEventListener("pointerdown", onDocDown, true);
      sizer?.disconnect();
      clearTimeout(wheelTimer);
      for (const t of hoverTimers.values()) clearTimeout(t);
    });
  });

  /** Where a new node goes: centred in the view, nudged so new nodes do not stack exactly. */
  function spot(s: { width: number; height: number }): { x: number; y: number } {
    const r = box.getBoundingClientRect();
    const c = toCanvas(view(), r.width / 2, r.height / 2);
    const n = shown().nodes.length % 6;
    return { x: c.x - s.width / 2 + n * 24, y: c.y - s.height / 2 + n * 24 };
  }

  function add(kind: NodeKind, items: Record<string, Json>) {
    const { block, id } = addNode(current(), kind, spot(NEW_NODE), items);
    commit(block);
    setSelected(id);
    closeAdd();
    // A new card opens ready to write in.
    if (kind === "card") {
      caretAt = null;
      setEditingCard(id);
    }
  }

  function closeAdd() {
    setAdding(false);
    setPending(null);
  }

  function closePopovers() {
    setPaletteOpen(null);
  }

  function addFrame() {
    const r = box.getBoundingClientRect();
    const { block, id } = addGroup(current(), toCanvas(view(), r.width / 4, r.height / 4), "Group");
    commit(block);
    setSelected(id);
    closeAdd();
  }

  function remove(id: string) {
    if (editingCard() === id) setEditingCard(null);
    commit(resnapEdges(removeItem(current(), id)));
    setSelected(null);
    closePopovers();
    box.focus({ preventScroll: true });
  }

  async function findNotes(q: string) {
    setPending({ kind: "note", q, results: [] });
    if (!q.trim()) return;
    try {
      const page = await api.search(q, null, 8);
      const p = pending();
      if (p?.kind === "note" && p.q === q) setPending({ ...p, results: page.rows.map((r) => ({ id: r.id, title: r.title })) });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const set = (id: string, name: string, value: Json | undefined) => commit(setItem(current(), id, name, value));

  // ---- card editing -------------------------------------------------------------------------

  function startEditing(id: string, at?: { x: number; y: number }) {
    if (!editing() || byId().get(id)?.kind !== "card") return;
    caretAt = at ?? null;
    closePopovers();
    setSelected(id);
    setEditingCard(id);
  }

  function stopEditing() {
    setEditingCard(null);
    caretAt = null;
  }

  /** Put the new editor's caret where the double-click was, else at the end of the card. */
  function focusEditor(editor: Editor) {
    const at = caretAt;
    caretAt = null;
    // At once, so keys typed straight after are not lost; then at the click, once laid out.
    if (!at) editor.commands.focus("end");
    requestAnimationFrame(() => {
      if (editor.isDestroyed) return;
      const pos = at ? editor.view.posAtCoords({ left: at.x, top: at.y }) : null;
      if (pos) editor.chain().setTextSelection(pos.pos).focus().run();
      else if (!editor.isFocused) editor.commands.focus("end");
    });
  }

  // ---- pointer gestures -------------------------------------------------------------------

  function capture(e: PointerEvent) {
    try {
      box.setPointerCapture(e.pointerId);
    } catch {
      // A synthetic pointer has nothing to capture; its events still arrive here.
    }
  }

  /** The canvas point under a pointer event. */
  function pointOf(e: PointerEvent): Point {
    const r = box.getBoundingClientRect();
    return toCanvas(view(), e.clientX - r.left, e.clientY - r.top);
  }

  /** Whether this press is the second of a double-click on the same thing. */
  function pressedTwice(key: string, e: PointerEvent): boolean {
    const now = performance.now();
    const twice =
      !!lastPress &&
      lastPress.key === key &&
      now - lastPress.t < DOUBLE_MS &&
      Math.hypot(e.clientX - lastPress.x, e.clientY - lastPress.y) < DOUBLE_SLOP;
    lastPress = twice ? null : { key, t: now, x: e.clientX, y: e.clientY };
    return twice;
  }

  function onBackgroundDown(e: PointerEvent) {
    if (drag || e.button !== 0) return;
    // Links, buttons and fields inside nodes keep their clicks.
    if (isInteractive(e.target)) return;
    e.preventDefault();
    closeAdd();
    closePopovers();
    box.focus({ preventScroll: true });
    capture(e);
    drag = { kind: "pan", pointer: e.pointerId, sx: e.clientX, sy: e.clientY, from: view(), moved: false };
  }

  /** The note a note node shows opens in a background tab on a middle-click or Ctrl/Cmd+click. */
  function openInNewTab(e: PointerEvent, id: string): boolean {
    const node = byId().get(id);
    const note = node?.kind === "note" ? node.block.fields?.note : undefined;
    if (typeof note !== "string" || !wantsNewTab(e)) return false;
    // A link inside the node, as in a query view's cards, opens by itself.
    if (e.target instanceof Element && e.target.closest("a")) return false;
    e.preventDefault();
    e.stopPropagation();
    openInBackground(`/notes/${note}`);
    return true;
  }

  function onItemDown(e: PointerEvent, id: string) {
    if (drag) return;
    if (openInNewTab(e, id)) return;
    if (!editing() || e.button !== 0) return;
    // The card being edited keeps its presses for its editor.
    if (editingCard() === id) {
      e.stopPropagation();
      return;
    }
    e.stopPropagation();
    if (editingCard()) stopEditing();
    closeAdd();
    if (paletteOpen() !== id) closePopovers();
    if (isInteractive(e.target)) {
      setSelected(id);
      return;
    }
    e.preventDefault();
    box.focus({ preventScroll: true });
    setSelected(id);
    if (pressedTwice(`item:${id}`, e) && byId().get(id)?.kind === "card") {
      startEditing(id, { x: e.clientX, y: e.clientY });
      return;
    }
    capture(e);
    drag = { kind: "move", pointer: e.pointerId, sx: e.clientX, sy: e.clientY, id, moved: false };
  }

  /** A press on a group's label: a double-click renames it, else it is a press on the group. */
  function onLabelDown(e: PointerEvent, id: string) {
    if (editing() && e.button === 0 && !drag && pressedTwice(`label:${id}`, e)) {
      e.stopPropagation();
      e.preventDefault();
      setSelected(id);
      setRenaming(id);
      return;
    }
    // The label's first press counts towards its own double-click, not the group frame's.
    const press = lastPress;
    onItemDown(e, id);
    lastPress = press;
  }

  function onResizeDown(e: PointerEvent, id: string, w: number, h: number) {
    if (!editing() || drag || e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    setSelected(id);
    capture(e);
    drag = { kind: "resize", pointer: e.pointerId, sx: e.clientX, sy: e.clientY, id, width: w, height: h };
  }

  /** Start drawing an edge from port `side` of node `id`. */
  function onPortDown(e: PointerEvent, id: string, side: Side) {
    if (!editing() || drag || e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    closeAdd();
    stopEditing();
    box.focus({ preventScroll: true });
    capture(e);
    drag = { kind: "link", pointer: e.pointerId, from: id, side };
    setLinking({ from: id, side, at: pointOf(e) });
  }

  /** The topmost node under canvas point `at`, other than `except`. */
  function nodeAt(at: Point, except: string): CanvasNode | undefined {
    return [...shown().nodes].reverse().find((n) => n.id !== except && contains(n, at.x, at.y));
  }

  function onPointerMove(e: PointerEvent) {
    const d = drag;
    if (!d || e.pointerId !== d.pointer) return;
    if (d.kind === "link") {
      setLinking({ from: d.from, side: d.side, at: pointOf(e) });
      return;
    }
    const zoom = view().zoom;
    const dx = e.clientX - d.sx;
    const dy = e.clientY - d.sy;
    if (d.kind === "pan") {
      if (Math.hypot(dx, dy) > CLICK_SLOP) d.moved = true;
      setView({ ...d.from, x: d.from.x - dx / zoom, y: d.from.y - dy / zoom });
    } else if (d.kind === "move") {
      if (Math.hypot(dx, dy) > CLICK_SLOP) d.moved = true;
      if (d.moved) setPreview(resnapEdges(moveItem(current(), d.id, dx / zoom, dy / zoom)));
    } else {
      setPreview(resnapEdges(resizeItem(current(), d.id, d.width + dx / zoom, d.height + dy / zoom)));
    }
  }

  function onPointerUp(e: PointerEvent) {
    const d = drag;
    if (!d || e.pointerId !== d.pointer) return;
    drag = null;
    if (d.kind === "link") {
      // Dropped on another node: join them. Dropped on empty canvas: nothing.
      const target = nodeAt(pointOf(e), d.from);
      setLinking(null);
      if (target) {
        const { block, id } = addEdge(current(), d.from, target.id);
        if (id) {
          commit(resnapEdges(block));
          setSelected(id);
        }
      }
      return;
    }
    if (d.kind === "pan") {
      if (d.moved) saveView();
      else if (editing()) setSelected(null);
      return;
    }
    const p = preview();
    if (p) commit(p);
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape" && (adding() || linking())) {
      e.preventDefault();
      closeAdd();
      return;
    }
    if (!editing()) return;
    const target = e.target as HTMLElement;
    // Keys inside the card being edited belong to its editor, which reports Escape itself.
    if (target !== box) return;
    const id = selected();
    if (id && (e.key === "Delete" || e.key === "Backspace")) {
      e.preventDefault();
      remove(id);
    } else if (id && e.key === "Enter" && byId().get(id)?.kind === "card") {
      e.preventDefault();
      startEditing(id);
    } else if (e.key === "Escape") {
      if (paletteOpen()) closePopovers();
      else setSelected(null);
    }
  }

  /**
   * Ports show on the node under the pointer, and on the selected one. A node's ports stay a
   * moment after the pointer leaves it, so the pointer can cross the node's edge, or a node
   * overlapping it, to reach a port.
   */
  function hover(id: string, over: boolean) {
    clearTimeout(hoverTimers.get(id));
    hoverTimers.delete(id);
    if (over) {
      if (!hovered().includes(id)) setHovered([...hovered(), id]);
    } else {
      hoverTimers.set(
        id,
        setTimeout(() => {
          hoverTimers.delete(id);
          setHovered(hovered().filter((x) => x !== id));
        }, 400),
      );
    }
  }

  // ---- the height handle -------------------------------------------------------------------

  function setHeight(h: number) {
    commit(withHeight(current(), clampHeight(h)));
  }

  // ---- rendering ----------------------------------------------------------------------------

  const world = () => {
    const v = view();
    return { transform: `translate(${-v.x * v.zoom}px, ${-v.y * v.zoom}px) scale(${v.zoom})` };
  };

  /** The background grid follows the view, so panning visibly moves the canvas. */
  const paper = () => {
    const v = view();
    const step = 32 * v.zoom;
    return {
      "background-size": `${step}px ${step}px`,
      "background-position": `${-v.x * v.zoom}px ${-v.y * v.zoom}px`,
      height: `${height()}px`,
    };
  };

  /** A canvas point on screen, relative to the view's top-left. */
  const toScreen = (p: Point): Point => {
    const v = view();
    return { x: (p.x - v.x) * v.zoom, y: (p.y - v.y) * v.zoom };
  };

  // Before the view has a size, as in a test without layout, everything counts as on screen.
  const onScreen = (p: Point) => size().width === 0 || p.x >= -8 && p.y >= -8 && p.x <= size().width + 8 && p.y <= size().height + 8;

  /** Nodes whose ports show: the hovered one, the selected one, and the one an edge starts from. */
  const portNodes = createMemo(() => {
    if (!editing()) return [];
    const ids = new Set([...hovered(), selected(), linking()?.from].filter((x): x is string => !!x));
    return [...ids].filter((id) => byId().has(id) && id !== editingCard());
  });

  /** The live route of an edge being drawn, to the node under the pointer or to the pointer. */
  const linkRoute = (): Point[] | null => {
    const l = linking();
    const from = l ? byId().get(l.from) : undefined;
    if (!l || !from) return null;
    const target = nodeAt(l.at, l.from);
    if (target) {
      const { fromSide, toSide } = closestSides(from, target);
      return elbowRoute(portPoint(from, fromSide), fromSide, portPoint(target, toSide), toSide);
    }
    const cursor = { x: l.at.x, y: l.at.y, width: 0, height: 0 };
    return elbowRoute(portPoint(from, l.side), l.side, l.at, nearestSide(cursor, from), 0);
  };

  const routeOf = (edge: CanvasEdge): Point[] | null => {
    const a = byId().get(edge.from);
    const b = byId().get(edge.to);
    return a && b ? edgeRoute(a, b, edge.fromSide, edge.toSide) : null;
  };

  function NodeBox(p: { id: string }) {
    const node = () => byId().get(p.id) as CanvasNode;
    const isEditing = () => editingCard() === p.id;
    return (
      <Show when={byId().get(p.id)}>
        <div
          class="canvas-node"
          classList={{
            selected: selected() === p.id,
            "editing-card": isEditing(),
            "link-target": !!linking() && linking()!.from !== p.id && nodeAt(linking()!.at, linking()!.from)?.id === p.id,
            [`canvas-node-${node().kind}`]: true,
          }}
          data-testid="canvas-node"
          data-id={p.id}
          data-kind={node().kind}
          data-colour={node().colour ?? undefined}
          style={{
            left: `${node().x}px`,
            top: `${node().y}px`,
            width: `${node().width}px`,
            height: `${node().height}px`,
            "--item-colour": paletteCss(node().colour),
          }}
          onPointerDown={(e) => onItemDown(e, p.id)}
          onMouseDown={(e) => {
            // A middle press would start the browser's autoscroll.
            if (e.button === 1 && node().kind === "note") e.preventDefault();
          }}
          onPointerEnter={() => hover(p.id, true)}
          onPointerLeave={() => hover(p.id, false)}
        >
          <div class="canvas-node-body">
            <Show
              when={isEditing()}
              fallback={props.body ? props.body(node()) : <NodeBody node={node()} />}
            >
              <div class="canvas-card-editor">
                <ContentEditor
                  blocks={node().block.blocks ?? []}
                  label="Card content"
                  exclude={CARD_EXCLUDES}
                  onChange={(blocks) => commit(setNodeBlocks(current(), p.id, blocks))}
                  onReady={(h) => focusEditor(h.editor)}
                  onEscape={() => {
                    stopEditing();
                    setSelected(p.id);
                    box.focus({ preventScroll: true });
                  }}
                />
              </div>
            </Show>
          </div>
          <Show when={editing() && selected() === p.id}>
            <span
              class="canvas-resize"
              aria-hidden="true"
              onPointerDown={(e) => onResizeDown(e, p.id, node().width, node().height)}
            />
          </Show>
        </div>
      </Show>
    );
  }

  /** The four ports of a node, on its sides, from which an edge is dragged out. */
  function Ports(p: { id: string }) {
    const node = () => byId().get(p.id);
    // Ports keep their size on screen at any zoom.
    const portSize = () => 14 / view().zoom;
    return (
      <Show when={node()}>
        {(n) => (
          <For each={SIDES}>
            {(side) => {
              const at = () => portPoint(n(), side);
              return (
                <span
                  class="canvas-port"
                  classList={{ active: linking()?.from === p.id && linking()?.side === side }}
                  data-testid="canvas-port"
                  data-node={p.id}
                  data-side={side}
                  title="Drag to another node to connect"
                  aria-hidden="true"
                  style={{
                    left: `${at().x - portSize() / 2}px`,
                    top: `${at().y - portSize() / 2}px`,
                    width: `${portSize()}px`,
                    height: `${portSize()}px`,
                    "border-width": `${2 / view().zoom}px`,
                  }}
                  onPointerEnter={() => hover(p.id, true)}
                  onPointerLeave={() => hover(p.id, false)}
                  onPointerDown={(e) => onPortDown(e, p.id, side)}
                />
              );
            }}
          </For>
        )}
      </Show>
    );
  }

  /** The colour swatch of a cluster, and the palette it opens. */
  function Swatch(p: { id: string; colour: string | null; up?: boolean }) {
    const open = () => paletteOpen() === p.id;
    const choose = (c: string | null) => {
      set(p.id, "colour", colourItem(c));
      setPaletteOpen(null);
    };
    return (
      <span class="canvas-swatch-wrap">
        <button
          type="button"
          class="canvas-action canvas-swatch-button"
          aria-label="Colour"
          title={`Colour: ${colourLabel(p.colour)}`}
          aria-haspopup="true"
          aria-expanded={open()}
          onClick={() => setPaletteOpen(open() ? null : p.id)}
        >
          <span class="canvas-swatch" classList={{ none: !p.colour }} style={{ background: paletteCss(p.colour) }} />
        </button>
        <Show when={open()}>
          <div
            class="canvas-palette"
            classList={{ up: !!p.up }}
            role="group"
            aria-label="Palette"
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                setPaletteOpen(null);
                box.focus({ preventScroll: true });
              }
            }}
          >
            <button
              type="button"
              class="canvas-palette-item"
              aria-label="No colour"
              title="No colour"
              aria-pressed={!p.colour}
              onClick={() => choose(null)}
              ref={(el) => queueMicrotask(() => el.focus({ preventScroll: true }))}
            >
              <span class="canvas-swatch none" />
            </button>
            <For each={PALETTE_COLOURS}>
              {(c) => (
                <button
                  type="button"
                  class="canvas-palette-item"
                  aria-label={colourLabel(c)}
                  title={colourLabel(c)}
                  aria-pressed={p.colour === c}
                  data-colour={c}
                  onClick={() => choose(c)}
                >
                  <span class="canvas-swatch" style={{ background: paletteCss(c) }} />
                </button>
              )}
            </For>
          </div>
        </Show>
      </span>
    );
  }

  function DeleteButton(p: { id: string; what: string }) {
    return (
      <button type="button" class="canvas-action danger" aria-label="Delete" title={`Delete ${p.what}`} onClick={() => remove(p.id)}>
        <TrashIcon />
      </button>
    );
  }

  /** Where a cluster sits for a box: just above its top-right corner, else inside it. */
  const clusterAt = (b: { x: number; y: number; width: number }) => {
    const corner = toScreen({ x: b.x + b.width, y: b.y });
    const right = size().width > 0 ? Math.min(corner.x, size().width) : corner.x;
    const above = corner.y >= 40;
    return { corner, style: { left: `${right}px`, top: `${above ? corner.y - 6 : Math.max(corner.y, 0) + 6}px` }, above };
  };

  /** The buttons of the selected node or group, at its top-right corner. */
  function ItemCluster() {
    const item = createMemo(() => {
      const id = selected();
      if (!id || !editing() || captioning()) return null;
      const n = byId().get(id);
      if (n) return { id, box: n as { x: number; y: number; width: number }, colour: n.colour, note: n.kind === "note" ? n.block.fields?.note : undefined, what: n.kind === "note" ? "note" : "card" };
      const g = shown().groups.find((x) => x.id === id);
      if (g) return { id, box: g, colour: g.colour, note: undefined, what: "group" };
      return null;
    });
    return (
      <Show when={item()}>
        {(it) => {
          const at = () => clusterAt(it().box);
          return (
            <Show when={onScreen(at().corner)}>
              <div
                class="canvas-cluster canvas-item-cluster"
                classList={{ above: at().above }}
                role="toolbar"
                aria-label={`${it().what === "group" ? "Group" : it().what === "note" ? "Note" : "Card"} actions`}
                data-testid="canvas-cluster"
                style={at().style}
                onPointerDown={(e) => e.stopPropagation()}
              >
                <Show when={typeof it().note === "string"}>
                  <button
                    type="button"
                    class="canvas-action"
                    aria-label="Open"
                    title="Open the note in a new tab"
                    onClick={() => openTab(`/notes/${it().note as string}`)}
                  >
                    <OpenIcon />
                  </button>
                </Show>
                <Swatch id={it().id} colour={it().colour} />
                <DeleteButton id={it().id} what={it().what} />
              </div>
            </Show>
          );
        }}
      </Show>
    );
  }

  /** The buttons of the selected edge, at its midpoint. */
  function EdgeCluster() {
    const item = createMemo(() => {
      const id = selected();
      if (!id || !editing()) return null;
      const edge = shown().edges.find((x) => x.id === id);
      const pts = edge ? routeOf(edge) : null;
      return edge && pts ? { edge, pts } : null;
    });
    return (
      <Show when={item()}>
        {(it) => {
          const mid = () => toScreen(midpoint(it().pts));
          const angle = () => routeDirection(it().pts);
          // Below the line when there is no room above it.
          const below = () => mid().y < 64;
          return (
            <Show when={onScreen(mid())}>
              <Show when={captioning() !== it().edge.id}>
                <div
                  class="canvas-cluster canvas-edge-cluster"
                  classList={{ below: below() }}
                  role="toolbar"
                  aria-label="Connection actions"
                  data-testid="canvas-edge-cluster"
                  style={{ left: `${mid().x}px`, top: `${mid().y}px` }}
                  onPointerDown={(e) => e.stopPropagation()}
                >
                  <Swatch id={it().edge.id} colour={it().edge.colour} up={!below() && mid().y > size().height - 150} />
                  <span class="canvas-arrows" role="group" aria-label="Arrows">
                    <For each={ARROWS}>
                      {(a) => (
                        <button
                          type="button"
                          class="canvas-action canvas-arrow-choice"
                          aria-label={ARROW_LABELS[a]}
                          title={ARROW_LABELS[a]}
                          aria-pressed={it().edge.arrow === a}
                          data-arrow={a}
                          onClick={() => set(it().edge.id, "arrow", arrowItem(a))}
                        >
                          <ArrowIcon arrow={a} angle={angle()} />
                        </button>
                      )}
                    </For>
                  </span>
                  <button
                    type="button"
                    class="canvas-action"
                    aria-label="Edit caption"
                    title="Edit caption"
                    onClick={() => setCaptioning(it().edge.id)}
                  >
                    <CaptionIcon />
                  </button>
                  <DeleteButton id={it().edge.id} what="connection" />
                </div>
              </Show>
              <Show when={captioning() === it().edge.id}>
                <input
                  class="canvas-caption-input"
                  aria-label="Caption"
                  placeholder="Caption"
                  style={{ left: `${mid().x}px`, top: `${mid().y}px` }}
                  ref={(el) => startTyping(el, untrack(() => it().edge.label))}
                  onInput={(e) => (typed = e.currentTarget.value)}
                  onPointerDown={(e) => e.stopPropagation()}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      finishCaption(it().edge.id, e.currentTarget.value);
                    } else if (e.key === "Escape") {
                      e.preventDefault();
                      e.stopPropagation();
                      setCaptioning(null);
                      box.focus({ preventScroll: true });
                    }
                  }}
                  onBlur={(e) => {
                    if (captioning() === it().edge.id) finishCaption(it().edge.id, e.currentTarget.value);
                  }}
                />
              </Show>
            </Show>
          );
        }}
      </Show>
    );
  }

  function finishCaption(id: string, value: string) {
    setCaptioning(null);
    const text = value.trim();
    const edge = shown().edges.find((x) => x.id === id);
    if (edge && edge.label !== text) set(id, "label", text || undefined);
    box.focus({ preventScroll: true });
  }

  function finishRename(id: string, value: string) {
    setRenaming(null);
    const group = shown().groups.find((g) => g.id === id);
    if (group && group.label !== value.trim()) set(id, "label", value.trim() || undefined);
    box.focus({ preventScroll: true });
  }

  return (
    <div class="canvas" data-testid="canvas" aria-label={props.label} role="group">
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <div class="canvas-stage">
        <div
          ref={box}
          class="canvas-view"
          classList={{ editing: editing(), linking: !!linking() }}
          style={paper()}
          tabindex={0}
          data-testid="canvas-view"
          data-node-view-interactive=""
          data-x={view().x}
          data-y={view().y}
          data-zoom={view().zoom}
          onPointerDown={onBackgroundDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          onKeyDown={onKeyDown}
          // The editor's block node is draggable; working on the canvas must not start a drag. A
          // card's own editor drags its blocks.
          onDragStart={(e) => {
            if (!(e.target instanceof Element && e.target.closest(".editing-card"))) e.preventDefault();
          }}
        >
          <div class="canvas-world" style={world()}>
            <For each={groupIds()}>
              {(id) => (
                <Show when={groupById().get(id)}>
                  {(g) => (
                    <div
                      class="canvas-group"
                      classList={{ selected: selected() === id }}
                      data-testid="canvas-group"
                      data-id={id}
                      data-colour={g().colour ?? undefined}
                      style={{
                        left: `${g().x}px`,
                        top: `${g().y}px`,
                        width: `${g().width}px`,
                        height: `${g().height}px`,
                        "--item-colour": paletteCss(g().colour),
                      }}
                      // The whole frame selects and moves the group. Nodes draw above groups, so a
                      // node inside one still takes its own clicks.
                      onPointerDown={(e) => onItemDown(e, id)}
                    >
                      <Show
                        when={renaming() === id}
                        fallback={
                          <span
                            class="canvas-group-label"
                            data-testid="canvas-group-label"
                            title={editing() ? "Double-click to rename" : undefined}
                            onPointerDown={(e) => onLabelDown(e, id)}
                          >
                            {g().label || id}
                          </span>
                        }
                      >
                        <input
                          class="canvas-group-label canvas-group-rename"
                          aria-label="Group label"
                          ref={(el) => startTyping(el, untrack(() => g().label))}
                          onInput={(e) => (typed = e.currentTarget.value)}
                          onPointerDown={(e) => e.stopPropagation()}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault();
                              finishRename(id, e.currentTarget.value);
                            } else if (e.key === "Escape") {
                              e.preventDefault();
                              e.stopPropagation();
                              setRenaming(null);
                              box.focus({ preventScroll: true });
                            }
                          }}
                          onBlur={(e) => {
                            if (renaming() === id) finishRename(id, e.currentTarget.value);
                          }}
                        />
                      </Show>
                      <Show when={editing() && selected() === id}>
                        <span class="canvas-resize" aria-hidden="true" onPointerDown={(e) => onResizeDown(e, id, g().width, g().height)} />
                      </Show>
                    </div>
                  )}
                </Show>
              )}
            </For>
            <svg class="canvas-edges" width="1" height="1">
              <For each={shown().edges}>
                {(edge) => (
                  <Show when={routeOf(edge)}>
                    {(pts) => {
                      const ends = () => arrowEnds(edge.arrow);
                      const n = () => pts().length;
                      return (
                        <g
                          class="canvas-edge"
                          classList={{ selected: selected() === edge.id }}
                          data-testid="canvas-edge"
                          data-id={edge.id}
                          data-arrow={edge.arrow}
                          data-colour={edge.colour ?? undefined}
                          data-points={pointsAttr(pts())}
                          style={{ color: paletteCss(edge.colour) }}
                          onPointerDown={(ev) => onItemDown(ev as unknown as PointerEvent, edge.id)}
                        >
                          <path class="canvas-edge-hit" d={roundedPath(pts())} />
                          <Show when={selected() === edge.id}>
                            <path class="canvas-edge-halo" d={roundedPath(pts())} />
                          </Show>
                          <path class="canvas-edge-line" d={roundedPath(trimRoute(pts(), ends().start, ends().end))} />
                          <Show when={ends().start}>
                            <polygon class="canvas-arrowhead" data-testid="canvas-arrowhead" data-end="start" points={arrowHead(pts()[1]!, pts()[0]!)} />
                          </Show>
                          <Show when={ends().end}>
                            <polygon
                              class="canvas-arrowhead"
                              data-testid="canvas-arrowhead"
                              data-end="end"
                              points={arrowHead(pts()[n() - 2]!, pts()[n() - 1]!)}
                            />
                          </Show>
                        </g>
                      );
                    }}
                  </Show>
                )}
              </For>
              <Show when={linkRoute()}>
                {(pts) => (
                  <g class="canvas-edge-preview-group">
                    <path class="canvas-edge-preview" data-testid="canvas-edge-preview" d={roundedPath(trimRoute(pts(), false, true))} />
                    <Show when={pts().length >= 2}>
                      <polygon class="canvas-preview-arrowhead" points={arrowHead(pts()[pts().length - 2]!, pts()[pts().length - 1]!)} />
                    </Show>
                  </g>
                )}
              </Show>
            </svg>
            {/* Captions sit on their lines, with a background the line does not show through. */}
            <For each={shown().edges.filter((e) => e.label)}>
              {(edge) => (
                <Show when={routeOf(edge)}>
                  {(pts) => (
                    <Show when={captioning() !== edge.id}>
                      <span
                        class="canvas-edge-label"
                        classList={{ selected: selected() === edge.id }}
                        data-testid="canvas-edge-label"
                        data-edge={edge.id}
                        data-colour={edge.colour ?? undefined}
                        style={{
                          left: `${midpoint(pts()).x}px`,
                          top: `${midpoint(pts()).y}px`,
                          "--item-colour": paletteCss(edge.colour),
                        }}
                        title={editing() && selected() === edge.id ? "Click to edit the caption" : undefined}
                        onPointerDown={(e) => {
                          if (editing() && e.button === 0 && selected() === edge.id) {
                            e.stopPropagation();
                            e.preventDefault();
                            setCaptioning(edge.id);
                            return;
                          }
                          onItemDown(e, edge.id);
                        }}
                      >
                        {edge.label}
                      </span>
                    </Show>
                  )}
                </Show>
              )}
            </For>
            <For each={nodeIds()}>{(id) => <NodeBox id={id} />}</For>
            <For each={portNodes()}>{(id) => <Ports id={id} />}</For>
          </div>
        </div>
        <div class="canvas-overlay" style={{ height: `${height()}px` }}>
          <ItemCluster />
          <EdgeCluster />
        </div>
        <div class="canvas-corner">
          <Show when={zoomOpen()}>
            <div class="canvas-zoom" role="group" aria-label="Zoom">
              <button type="button" aria-label="Zoom out" title="Zoom out" onClick={() => (zoomBy(1 / ZOOM_STEP), saveView())}>
                −
              </button>
              <span class="canvas-zoom-level" data-testid="canvas-zoom">
                {Math.round(view().zoom * 100)}%
              </span>
              <button type="button" aria-label="Zoom in" title="Zoom in" onClick={() => (zoomBy(ZOOM_STEP), saveView())}>
                +
              </button>
              <button type="button" aria-label="Fit to content" title="Fit to content" onClick={fit}>
                Fit
              </button>
              <button type="button" aria-label="Zoom to 100%" title="Zoom to 100%" onClick={actualSize}>
                1:1
              </button>
            </div>
          </Show>
          <button
            type="button"
            class="canvas-corner-button"
            aria-label="Zoom controls"
            title="Zoom controls"
            aria-pressed={zoomOpen()}
            onClick={toggleZoom}
          >
            <MagnifierIcon />
          </button>
          <Show when={editing()}>
            <button
              type="button"
              class="canvas-corner-button"
              aria-label="Add to canvas"
              title="Add to canvas"
              aria-expanded={adding()}
              onClick={() => (adding() ? closeAdd() : (stopEditing(), setAdding(true)))}
            >
              +
            </button>
          </Show>
        </div>
        <Show when={adding()}>
          <div
            class="canvas-popup"
            role="dialog"
            aria-label="Add to canvas"
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                closeAdd();
                box.focus({ preventScroll: true });
              }
            }}
          >
            <Show
              when={pending()}
              fallback={
                <>
                  <button type="button" class="canvas-popup-main" onClick={() => add("card", {})}>
                    <strong>Card</strong>
                    <span class="muted">Text, images, queries and other blocks</span>
                  </button>
                  <button type="button" class="canvas-popup-main" onClick={() => setPending({ kind: "note", q: "", results: [] })}>
                    <strong>Note</strong>
                    <span class="muted">A note, shown as its card</span>
                  </button>
                  <p class="canvas-popup-more">More</p>
                  <div class="canvas-popup-row">
                    <button type="button" onClick={addFrame}>
                      Group
                    </button>
                  </div>
                </>
              }
            >
              {(p) => (
                <>
                  <input
                    type="search"
                    aria-label="Find a note"
                    placeholder="Find a note"
                    value={p().q}
                    onInput={(e) => void findNotes(e.currentTarget.value)}
                    ref={(el) => queueMicrotask(() => el.focus())}
                  />
                  <ul class="canvas-results" aria-label="Notes found">
                    <For each={p().results}>
                      {(r) => (
                        <li>
                          <button type="button" onClick={() => add("note", { note: r.id })}>
                            {r.title || r.id}
                          </button>
                        </li>
                      )}
                    </For>
                  </ul>
                  <button type="button" onClick={() => setPending(null)}>
                    Back
                  </button>
                </>
              )}
            </Show>
          </div>
        </Show>
        <Show when={editing()}>
          <HeightHandle
            label="Canvas height"
            height={stored().height ?? CANVAS_HEIGHT.initial}
            range={CANVAS_HEIGHT}
            onPreview={setHeightPreview}
            onChange={setHeight}
            testid="canvas-height-handle"
            id={`${uid}-height`}
          />
        </Show>
      </div>
    </div>
  );
}
