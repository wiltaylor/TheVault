// The one canvas component (D15): a canvas content block and a canvas field both render through
// it. The view pans by dragging the background and zooms with the wheel or the zoom buttons.
// While editing, nodes are added from the toolbar, moved by dragging, resized from their corner,
// joined by edges in connect mode, and changed in the inspector; every change, the view included,
// reports the whole new block. Without `onChange` the canvas is shown, and the view moves without
// being saved. `body` draws a node in place of its kind's own body, which is how a canvas query
// view shows its results as cards.

import { type JSX, For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import type { Block, Json } from "../api";
import { FilePicker, upload } from "../blocks/MediaBlocks";
import { PALETTE } from "../editor/markup";
import { api } from "../api";
import {
  type Canvas,
  type CanvasNode,
  type NodeKind,
  type Viewport,
  NEW_NODE,
  addEdge,
  addGroup,
  addNode,
  canvasOf,
  edgeEnds,
  moveItem,
  removeItem,
  resizeItem,
  setItem,
  toCanvas,
  withViewport,
  zoomAbout,
} from "./canvas";
import { NodeBody } from "./CanvasNodes";

/** Zoom steps for the buttons, and for one notch of the wheel. */
const ZOOM_STEP = 1.25;
const WHEEL_STEP = 1.1;

/** Wait this long after the last wheel notch before saving the view. */
const WHEEL_SETTLE_MS = 250;

/** A pointer that moves less than this, in screen pixels, clicked rather than dragged. */
const CLICK_SLOP = 3;

const PALETTE_HEX: Record<string, string> = {
  gray: "#6b7280",
  brown: "#92400e",
  orange: "#ea580c",
  yellow: "#ca8a04",
  green: "#16a34a",
  blue: "#2563eb",
  purple: "#9333ea",
  pink: "#db2777",
  red: "#dc2626",
};

const colourOf = (c: string | null) => (c ? PALETTE_HEX[c] : undefined);

type Drag =
  | { kind: "pan"; pointer: number; sx: number; sy: number; from: Viewport; moved: boolean }
  | { kind: "move"; pointer: number; sx: number; sy: number; id: string; moved: boolean }
  | { kind: "resize"; pointer: number; sx: number; sy: number; id: string; width: number; height: number };

/** A form the toolbar opens before adding a node that needs a value first. */
type Pending = { kind: "link"; url: string } | { kind: "note"; q: string; results: { id: string; title: string }[] };

let surfaces = 0;

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
  const [connecting, setConnecting] = createSignal(false);
  const [connectFrom, setConnectFrom] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<Pending | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  let box!: HTMLDivElement;
  let drag: Drag | null = null;
  let wheelTimer: ReturnType<typeof setTimeout> | undefined;

  const editing = () => !!props.onChange;
  const shown = createMemo<Canvas>(() => {
    const p = preview();
    return p ? canvasOf(p) : stored();
  });
  const byId = createMemo(() => new Map(shown().nodes.map((n) => [n.id, n])));
  const nodeIds = createMemo(() => shown().nodes.map((n) => n.id), undefined, {
    equals: (a, b) => a.length === b.length && a.every((x, i) => x === b[i]),
  });

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

  onMount(() => {
    // Not a Solid handler: the wheel must be able to cancel the page's own scroll.
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey && !e.altKey && Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
      e.preventDefault();
      const r = box.getBoundingClientRect();
      zoomBy(e.deltaY < 0 ? WHEEL_STEP : 1 / WHEEL_STEP, e.clientX - r.left, e.clientY - r.top);
      clearTimeout(wheelTimer);
      wheelTimer = setTimeout(saveView, WHEEL_SETTLE_MS);
    };
    box.addEventListener("wheel", onWheel, { passive: false });
    onCleanup(() => {
      box.removeEventListener("wheel", onWheel);
      clearTimeout(wheelTimer);
    });
  });

  /** Where a new node goes: the middle of the view, nudged so new nodes do not stack exactly. */
  function spot(): { x: number; y: number } {
    const r = box.getBoundingClientRect();
    const c = toCanvas(view(), r.width / 2, r.height / 2);
    const n = shown().nodes.length % 6;
    return { x: c.x - NEW_NODE.width / 2 + n * 24, y: c.y - NEW_NODE.height / 2 + n * 24 };
  }

  function add(kind: NodeKind, items: Record<string, Json>) {
    const { block, id } = addNode(current(), kind, spot(), items);
    commit(block);
    setSelected(id);
    setPending(null);
  }

  function addFrame() {
    const r = box.getBoundingClientRect();
    const { block, id } = addGroup(current(), toCanvas(view(), r.width / 4, r.height / 4), "Group");
    commit(block);
    setSelected(id);
  }

  function remove(id: string) {
    commit(removeItem(current(), id));
    setSelected(null);
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

  async function addAsset(file: File) {
    setError(null);
    try {
      const asset = await upload(file);
      add("asset", { asset: asset.id });
    } catch (e) {
      setError(`Could not upload ${file.name}: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  // ---- pointer gestures -------------------------------------------------------------------

  function capture(e: PointerEvent) {
    try {
      box.setPointerCapture(e.pointerId);
    } catch {
      // A synthetic pointer has nothing to capture; its events still arrive here.
    }
  }

  function onBackgroundDown(e: PointerEvent) {
    if (drag || e.button !== 0) return;
    // Links, buttons and fields inside nodes keep their clicks.
    if (e.target instanceof Element && e.target.closest("a, button, input, textarea, select")) return;
    e.preventDefault();
    box.focus({ preventScroll: true });
    capture(e);
    drag = { kind: "pan", pointer: e.pointerId, sx: e.clientX, sy: e.clientY, from: view(), moved: false };
  }

  function onItemDown(e: PointerEvent, id: string, isNode: boolean) {
    if (!editing() || drag || e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    box.focus({ preventScroll: true });
    if (connecting() && isNode) {
      const from = connectFrom();
      if (!from) {
        setConnectFrom(id);
        setSelected(id);
      } else {
        const { block, id: edge } = addEdge(current(), from, id);
        if (edge) commit(block);
        setConnectFrom(null);
        setSelected(edge ?? id);
      }
      return;
    }
    setSelected(id);
    capture(e);
    drag = { kind: "move", pointer: e.pointerId, sx: e.clientX, sy: e.clientY, id, moved: false };
  }

  function onResizeDown(e: PointerEvent, id: string, w: number, h: number) {
    if (!editing() || drag || e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    setSelected(id);
    capture(e);
    drag = { kind: "resize", pointer: e.pointerId, sx: e.clientX, sy: e.clientY, id, width: w, height: h };
  }

  function onPointerMove(e: PointerEvent) {
    const d = drag;
    if (!d || e.pointerId !== d.pointer) return;
    const zoom = view().zoom;
    const dx = e.clientX - d.sx;
    const dy = e.clientY - d.sy;
    if (d.kind === "pan") {
      if (Math.hypot(dx, dy) > CLICK_SLOP) d.moved = true;
      setView({ ...d.from, x: d.from.x - dx / zoom, y: d.from.y - dy / zoom });
    } else if (d.kind === "move") {
      if (Math.hypot(dx, dy) > CLICK_SLOP) d.moved = true;
      if (d.moved) setPreview(moveItem(current(), d.id, dx / zoom, dy / zoom));
    } else {
      setPreview(resizeItem(current(), d.id, d.width + dx / zoom, d.height + dy / zoom));
    }
  }

  function onPointerUp(e: PointerEvent) {
    const d = drag;
    if (!d || e.pointerId !== d.pointer) return;
    drag = null;
    if (d.kind === "pan") {
      if (d.moved) saveView();
      else if (editing()) {
        setSelected(null);
        setConnectFrom(null);
      }
      return;
    }
    const p = preview();
    if (p) commit(p);
  }

  function onKeyDown(e: KeyboardEvent) {
    if (!editing()) return;
    const target = e.target as HTMLElement;
    if (target !== box) return;
    const id = selected();
    if (id && (e.key === "Delete" || e.key === "Backspace")) {
      e.preventDefault();
      remove(id);
    } else if (e.key === "Escape") {
      setConnecting(false);
      setConnectFrom(null);
      setSelected(null);
    }
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
    };
  };

  const selectedItem = () => {
    const id = selected();
    if (!id) return null;
    const c = shown();
    const node = c.nodes.find((n) => n.id === id);
    if (node) return { type: "node" as const, node };
    const group = c.groups.find((g) => g.id === id);
    if (group) return { type: "group" as const, group };
    const edge = c.edges.find((x) => x.id === id);
    if (edge) return { type: "edge" as const, edge };
    return null;
  };

  const set = (id: string, name: string, value: Json | undefined) => commit(setItem(current(), id, name, value));

  function NodeBox(p: { id: string }) {
    const node = () => byId().get(p.id) as CanvasNode;
    return (
      <Show when={byId().get(p.id)}>
        <div
          class="canvas-node"
          classList={{
            selected: selected() === p.id,
            "connect-from": connectFrom() === p.id,
            [`canvas-node-${node().kind}`]: true,
          }}
          data-testid="canvas-node"
          data-id={p.id}
          data-kind={node().kind}
          style={{
            left: `${node().x}px`,
            top: `${node().y}px`,
            width: `${node().width}px`,
            height: `${node().height}px`,
            "border-color": colourOf(node().colour),
          }}
          onPointerDown={(e) => onItemDown(e, p.id, true)}
        >
          <div class="canvas-node-body">
            {props.body ? props.body(node()) : <NodeBody node={node()} />}
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

  return (
    <div class="canvas" data-testid="canvas" aria-label={props.label} role="group">
      <div class="canvas-toolbar" role="toolbar" aria-label={`${props.label} tools`}>
        <Show when={editing()}>
          <button type="button" onClick={() => add("text", { text: "Text" })}>
            Add text
          </button>
          <button type="button" onClick={() => setPending({ kind: "note", q: "", results: [] })}>
            Add note
          </button>
          <button type="button" onClick={() => add("query", { query: "from * sort updated desc limit 20" })}>
            Add query
          </button>
          <button type="button" onClick={() => setPending({ kind: "link", url: "https://" })}>
            Add link
          </button>
          <FilePicker label="Add file" text="Add file" onFile={(f) => void addAsset(f)} />
          <button type="button" onClick={addFrame}>
            Add group
          </button>
          <button
            type="button"
            aria-pressed={connecting()}
            title="Click one node, then another, to join them"
            onClick={() => {
              setConnecting(!connecting());
              setConnectFrom(null);
            }}
          >
            Connect
          </button>
        </Show>
        <span class="canvas-zoom">
          <button type="button" aria-label="Zoom out" onClick={() => (zoomBy(1 / ZOOM_STEP), saveView())}>
            −
          </button>
          <span class="canvas-zoom-level" data-testid="canvas-zoom">
            {Math.round(view().zoom * 100)}%
          </span>
          <button type="button" aria-label="Zoom in" onClick={() => (zoomBy(ZOOM_STEP), saveView())}>
            +
          </button>
          <button
            type="button"
            onClick={() => {
              setView({ x: 0, y: 0, zoom: 1 });
              saveView();
            }}
          >
            Reset view
          </button>
        </span>
      </div>
      <Show when={pending()}>
        {(p) => (
          <div class="canvas-pending">
            <Show
              when={p().kind === "link"}
              fallback={
                <>
                  <input
                    type="search"
                    aria-label="Find a note"
                    placeholder="Find a note"
                    value={(p() as Extract<Pending, { kind: "note" }>).q}
                    onInput={(e) => void findNotes(e.currentTarget.value)}
                    ref={(el) => queueMicrotask(() => el.focus())}
                  />
                  <ul class="canvas-results" aria-label="Notes found">
                    <For each={(p() as Extract<Pending, { kind: "note" }>).results}>
                      {(r) => (
                        <li>
                          <button type="button" onClick={() => add("note", { note: r.id })}>
                            {r.title || r.id}
                          </button>
                        </li>
                      )}
                    </For>
                  </ul>
                </>
              }
            >
              <input
                type="url"
                aria-label="Link address"
                value={(p() as Extract<Pending, { kind: "link" }>).url}
                onInput={(e) => setPending({ kind: "link", url: e.currentTarget.value })}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    add("link", { url: (p() as Extract<Pending, { kind: "link" }>).url.trim() });
                  }
                }}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
              <button type="button" onClick={() => add("link", { url: (p() as Extract<Pending, { kind: "link" }>).url.trim() })}>
                Add this link
              </button>
            </Show>
            <button type="button" onClick={() => setPending(null)}>
              Cancel
            </button>
          </div>
        )}
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <div
        ref={box}
        class="canvas-view"
        classList={{ editing: editing(), connecting: connecting() }}
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
        // The editor's block node is draggable; working on the canvas must not start a drag.
        onDragStart={(e) => e.preventDefault()}
      >
        <div class="canvas-world" style={world()}>
          <For each={shown().groups}>
            {(g) => (
              <div
                class="canvas-group"
                classList={{ selected: selected() === g.id }}
                data-testid="canvas-group"
                data-id={g.id}
                style={{
                  left: `${g.x}px`,
                  top: `${g.y}px`,
                  width: `${g.width}px`,
                  height: `${g.height}px`,
                  "border-color": colourOf(g.colour),
                }}
              >
                <span class="canvas-group-label" onPointerDown={(e) => onItemDown(e, g.id, false)}>
                  {g.label || g.id}
                </span>
                <Show when={editing() && selected() === g.id}>
                  <span class="canvas-resize" aria-hidden="true" onPointerDown={(e) => onResizeDown(e, g.id, g.width, g.height)} />
                </Show>
              </div>
            )}
          </For>
          <svg class="canvas-edges" width="1" height="1">
            <defs>
              <marker id={`${uid}-arrow`} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse">
                <path d="M0,0 L10,5 L0,10 z" class="canvas-arrow" />
              </marker>
            </defs>
            <For each={shown().edges}>
              {(edge) => {
                const ends = () => {
                  const a = byId().get(edge.from);
                  const b = byId().get(edge.to);
                  return a && b ? edgeEnds(a, b, edge.fromSide, edge.toSide) : null;
                };
                return (
                  <Show when={ends()}>
                    {(e) => (
                      <g
                        class="canvas-edge"
                        classList={{ selected: selected() === edge.id }}
                        data-testid="canvas-edge"
                        data-id={edge.id}
                        style={{ color: colourOf(edge.colour) }}
                        onPointerDown={(ev) => onItemDown(ev as unknown as PointerEvent, edge.id, false)}
                      >
                        <line class="canvas-edge-hit" x1={e().a.x} y1={e().a.y} x2={e().b.x} y2={e().b.y} />
                        <line
                          class="canvas-edge-line"
                          x1={e().a.x}
                          y1={e().a.y}
                          x2={e().b.x}
                          y2={e().b.y}
                          marker-end={edge.arrow !== "none" ? `url(#${uid}-arrow)` : undefined}
                          marker-start={edge.arrow === "both" ? `url(#${uid}-arrow)` : undefined}
                        />
                        <Show when={edge.label}>
                          <text class="canvas-edge-label" x={(e().a.x + e().b.x) / 2} y={(e().a.y + e().b.y) / 2 - 6}>
                            {edge.label}
                          </text>
                        </Show>
                      </g>
                    )}
                  </Show>
                );
              }}
            </For>
          </svg>
          <For each={nodeIds()}>{(id) => <NodeBox id={id} />}</For>
        </div>
      </div>
      <Show when={editing() && selectedItem()}>
        {(item) => (
          <div class="canvas-inspector" role="group" aria-label="Selected item">
            <Show when={item().type === "node" && (item() as { node: CanvasNode }).node}>
              {(n) => (
                <>
                  <span class="muted">{n().kind} node</span>
                  <Show when={n().kind === "text"}>
                    <textarea
                      aria-label="Node text"
                      value={String(n().block.fields?.text ?? "")}
                      onChange={(e) => set(n().id, "text", e.currentTarget.value)}
                    />
                  </Show>
                  <Show when={n().kind === "link"}>
                    <input
                      type="url"
                      aria-label="Node link"
                      value={String(n().block.fields?.url ?? "")}
                      onChange={(e) => set(n().id, "url", e.currentTarget.value.trim())}
                    />
                  </Show>
                  <Show when={n().kind === "query"}>
                    <input
                      aria-label="Node query"
                      value={String(n().block.fields?.query ?? "")}
                      onChange={(e) => set(n().id, "query", e.currentTarget.value)}
                    />
                    <select
                      aria-label="Node view"
                      value={String(n().block.fields?.view ?? "cards")}
                      onChange={(e) => set(n().id, "view", e.currentTarget.value === "cards" ? undefined : e.currentTarget.value)}
                    >
                      <option value="cards">cards</option>
                      <option value="list">list</option>
                      <option value="table">table</option>
                    </select>
                  </Show>
                  <Show when={n().kind === "note" || n().kind === "query"}>
                    <input
                      aria-label="Node template"
                      placeholder="card"
                      value={String(n().block.fields?.template ?? "")}
                      onChange={(e) => set(n().id, "template", e.currentTarget.value.trim() || undefined)}
                    />
                  </Show>
                </>
              )}
            </Show>
            <Show when={item().type === "group" && (item() as { group: { id: string; label: string } }).group}>
              {(g) => (
                <input
                  aria-label="Group label"
                  value={g().label}
                  onChange={(e) => set(g().id, "label", e.currentTarget.value || undefined)}
                />
              )}
            </Show>
            <Show when={item().type === "edge" && (item() as { edge: { id: string; label: string; arrow: string } }).edge}>
              {(edge) => (
                <>
                  <input
                    aria-label="Edge label"
                    value={edge().label}
                    onChange={(e) => set(edge().id, "label", e.currentTarget.value || undefined)}
                  />
                  <select
                    aria-label="Arrow"
                    value={edge().arrow}
                    onChange={(e) => set(edge().id, "arrow", e.currentTarget.value === "end" ? undefined : e.currentTarget.value)}
                  >
                    <option value="end">end</option>
                    <option value="both">both</option>
                    <option value="none">none</option>
                  </select>
                </>
              )}
            </Show>
            <select
              aria-label="Colour"
              value={selectedColour(item())}
              onChange={(e) => {
                const id = selected();
                if (id) set(id, "colour", e.currentTarget.value || undefined);
              }}
            >
              <option value="">Default colour</option>
              <For each={PALETTE}>{(c) => <option value={c}>{c}</option>}</For>
            </select>
            <button type="button" class="danger" onClick={() => remove(selected()!)}>
              Delete
            </button>
          </div>
        )}
      </Show>
    </div>
  );
}

function selectedColour(item: { node: CanvasNode } | { group: { colour: string | null } } | { edge: { colour: string | null } }): string {
  const thing = "node" in item ? item.node : "group" in item ? item.group : item.edge;
  return thing.colour ?? "";
}
