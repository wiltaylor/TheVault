// The graph at the top of the right sidebar: the active pane's note in the middle, the notes it
// links to and that link to it around it, and the links among them. A node is a link to its note,
// so a click opens it in the active pane and Ctrl/middle-click opens a background tab (the
// workspace's document-wide link handling does both). Drag the background to pan, wheel or pinch
// to zoom. The graph loads again after every note write, and moves to its new layout when the note
// in the middle changes.

import { For, Show, createComputed, createEffect, createMemo, createSignal, createUniqueId, on, onCleanup, onMount, untrack } from "solid-js";
import { type GraphNode, type NoteGraph, api } from "../api";
import { changes } from "../changes";
import { TypeIcon } from "../components/Icon";
import { errorText } from "../pages/TypeEditor";
import { typeDef } from "../session";
import { NODE_R, type Line, layoutGraph, linesOf } from "./layout";

/** The panel's height, in pixels. */
export const GRAPH_HEIGHT = 240;
const MIN_ZOOM = 0.35;
const MAX_ZOOM = 4;
const TWEEN_MS = 320;
/** The longest label, in characters, before it is cut short. */
const LABEL_CHARS = 16;

interface Point {
  x: number;
  y: number;
}

interface View {
  x: number;
  y: number;
  k: number;
}

const HOME: View = { x: 0, y: 0, k: 1 };

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function easeOut(t: number): number {
  return 1 - (1 - t) ** 3;
}

function reducedMotion(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

export function shortLabel(title: string): string {
  const t = title.trim() || "Untitled";
  return t.length > LABEL_CHARS ? `${t.slice(0, LABEL_CHARS - 1).trimEnd()}…` : t;
}

function typeLabel(type: string): string {
  return typeDef(type)?.label ?? type;
}

/** How a neighbour relates to the note in the middle, in words. */
function relation(n: GraphNode): string {
  switch (n.direction) {
    case "out":
      return "linked from this note";
    case "in":
      return "links to this note";
    case "both":
      return "linked both ways";
    default:
      return "this note";
  }
}

/** A node's accessible name, such as "Task: Fix pump, links to this note". */
export function nodeName(n: GraphNode): string {
  return `${typeLabel(n.type)}: ${n.title.trim() || "Untitled"}, ${relation(n)}`;
}

function nodeColour(type: string): string {
  const c = typeDef(type)?.colour;
  return c ? `var(--palette-${c})` : "var(--accent)";
}

/** The graph of note `noteId`, drawn to the panel's width. */
export function GraphView(props: { noteId: string }) {
  const markers = createUniqueId();
  const [graph, setGraph] = createSignal<NoteGraph | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [width, setWidth] = createSignal(0);
  const [view, setView] = createSignal<View>(HOME);
  const [hovered, setHovered] = createSignal<string | null>(null);

  // Load the graph when the note changes, and again shortly after any note write.
  let loads = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  async function load(id: string) {
    const seq = ++loads;
    try {
      const g = await api.graph(id);
      if (seq !== loads) return;
      // An unchanged graph (most saves change no links) keeps its layout as it is.
      if (JSON.stringify(g) !== JSON.stringify(graph())) setGraph(g);
      setError(null);
    } catch (e) {
      if (seq === loads) setError(errorText(e));
    }
  }
  createEffect(on(() => props.noteId, (id) => void load(id)));
  createEffect(
    on(
      changes,
      () => {
        clearTimeout(timer);
        timer = setTimeout(() => void load(untrack(() => props.noteId)), 150);
      },
      { defer: true },
    ),
  );
  onCleanup(() => clearTimeout(timer));

  const layout = createMemo(() => {
    const g = graph();
    return g ? layoutGraph(g.id, g.nodes.slice(1)) : null;
  });
  const lines = createMemo<Line[]>(() => {
    const g = graph();
    return g ? linesOf(g.id, g.edges) : [];
  });
  const nodes = createMemo(() => graph()?.nodes ?? []);
  const centre = () => graph()?.id ?? "";

  /**
   * The scale that fits the whole layout in the panel, never above 1.3, and how far the rings are
   * stretched across to use a panel wider than it is tall. Stretching only moves nodes apart.
   */
  const fitting = createMemo(() => {
    const l = layout();
    const w = width();
    if (!l || w <= 0) return { s: 1, a: 1 };
    const padX = l.labelled ? 42 : 8;
    const padY = l.labelled ? 24 : 8;
    const hw = w / 2 - 6;
    const hh = GRAPH_HEIGHT / 2 - 6;
    const sy = hh / (l.height + padY);
    const a = Math.max(1, Math.min(1.8, (hw / Math.min(sy, 1.3) - padX) / l.width));
    return { s: Math.min(1.3, sy, hw / (l.width * a + padX)), a };
  });
  const fit = () => fitting().s;
  /** Where node `id` goes in the current layout, stretched to the panel. */
  function target(id: string): Point | undefined {
    const p = layout()?.nodes.get(id);
    return p && { x: p.x * fitting().a, y: p.y };
  }

  // The move from one layout to the next: where each node was, and how far along the move is.
  const [t, setT] = createSignal(1);
  let from = new Map<string, Point>();
  let fitFrom = 1;
  let viewFrom: View = HOME;
  let frame = 0;
  const e = () => easeOut(t());

  // What the panel last drew, where a change of layout starts from.
  const shown = new Map<string, Point>();
  let shownFit = 1;
  let shownView: View = HOME;

  function drawnAt(id: string): Point | undefined {
    const to = target(id);
    if (!to) return undefined;
    const was = from.get(id);
    const p = !was || t() >= 1 ? to : { x: lerp(was.x, to.x, e()), y: lerp(was.y, to.y, e()) };
    shown.set(id, p);
    return p;
  }
  const drawnFit = () => (shownFit = t() >= 1 ? fit() : lerp(fitFrom, fit(), e()));
  const drawnView = (): View => {
    const v = view();
    if (t() >= 1) return (shownView = v);
    return (shownView = { x: lerp(viewFrom.x, v.x, e()), y: lerp(viewFrom.y, v.y, e()), k: lerp(viewFrom.k, v.k, e()) });
  };
  /** Whether node `id` is new in this layout, so it fades in rather than moves. */
  const appearing = (id: string) => t() < 1 && !from.has(id);

  function tween(nextView?: View) {
    cancelAnimationFrame(frame);
    if (reducedMotion()) {
      setT(1);
      if (nextView) setView(nextView);
      return;
    }
    const start = performance.now();
    const step = (now: number) => {
      const p = Math.min(1, (now - start) / TWEEN_MS);
      setT(p);
      if (p < 1) frame = requestAnimationFrame(step);
    };
    setT(0);
    if (nextView) setView(nextView);
    frame = requestAnimationFrame(step);
  }
  onCleanup(() => cancelAnimationFrame(frame));

  // Move from what is on screen to the new layout. This runs before the new layout is drawn, so
  // `shown`, `shownFit` and `shownView` still hold what the panel shows.
  let lastCentre = "";
  createComputed(
    on(layout, (l, prev) => {
      if (!l) return;
      const recentred = lastCentre !== "" && lastCentre !== centre();
      lastCentre = centre();
      if (!prev) return;
      from = new Map([...shown].filter(([id]) => prev.nodes.has(id)));
      fitFrom = shownFit;
      viewFrom = shownView;
      tween(recentred ? HOME : undefined);
    }),
  );

  let box: HTMLDivElement | undefined;
  let svg: SVGSVGElement | undefined;
  onMount(() => {
    if (!box) return;
    setWidth(box.clientWidth);
    const ro = new ResizeObserver(() => setWidth(box!.clientWidth));
    ro.observe(box);
    onCleanup(() => ro.disconnect());
  });

  // Pan and zoom. `s` is a point in the panel, in pixels from its top left.
  const scale = () => drawnFit() * drawnView().k;
  function zoomAt(s: Point, k: number) {
    const v = view();
    const nk = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, k));
    const f = fit();
    const cx = width() / 2;
    const cy = GRAPH_HEIGHT / 2;
    // The layout point under `s` stays under it.
    const px = (s.x - cx - v.x) / (f * v.k);
    const py = (s.y - cy - v.y) / (f * v.k);
    setView({ k: nk, x: s.x - cx - px * f * nk, y: s.y - cy - py * f * nk });
  }
  function local(ev: { clientX: number; clientY: number }): Point {
    const r = svg!.getBoundingClientRect();
    return { x: ev.clientX - r.left, y: ev.clientY - r.top };
  }
  onMount(() => {
    if (!svg) return;
    const onWheel = (ev: WheelEvent) => {
      ev.preventDefault();
      const unit = ev.deltaMode === 1 ? 16 : 1;
      zoomAt(local(ev), view().k * Math.exp(-ev.deltaY * unit * 0.0015));
    };
    svg.addEventListener("wheel", onWheel, { passive: false });
    onCleanup(() => svg!.removeEventListener("wheel", onWheel));
  });

  const pointers = new Map<number, Point>();
  let pinch: { d: number; k: number } | null = null;
  function onPointerDown(ev: PointerEvent) {
    if ((ev.target as Element).closest(".graph-node") || ev.button !== 0) return;
    ev.preventDefault();
    svg!.setPointerCapture(ev.pointerId);
    pointers.set(ev.pointerId, local(ev));
    if (pointers.size === 2) {
      const [a, b] = [...pointers.values()] as [Point, Point];
      pinch = { d: Math.hypot(a.x - b.x, a.y - b.y), k: view().k };
    }
  }
  function onPointerMove(ev: PointerEvent) {
    const was = pointers.get(ev.pointerId);
    if (!was) return;
    const now = local(ev);
    pointers.set(ev.pointerId, now);
    if (pointers.size === 1) {
      const v = view();
      setView({ ...v, x: v.x + now.x - was.x, y: v.y + now.y - was.y });
    } else if (pinch && pointers.size === 2) {
      const [a, b] = [...pointers.values()] as [Point, Point];
      const d = Math.hypot(a.x - b.x, a.y - b.y);
      zoomAt({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }, (pinch.k * d) / Math.max(1, pinch.d));
    }
  }
  function onPointerUp(ev: PointerEvent) {
    pointers.delete(ev.pointerId);
    if (pointers.size < 2) pinch = null;
  }

  /** Whether the hover leaves `id` (a node) or the line `a`-`b` in full colour. */
  const lit = (id: string) => {
    const h = hovered();
    if (!h || h === id) return true;
    return lines().some((l) => (l.a === h && l.b === id) || (l.b === h && l.a === id));
  };
  const lineLit = (l: Line) => {
    const h = hovered();
    return !h || l.a === h || l.b === h;
  };
  const showIcons = () => scale() * NODE_R >= 7;
  // The middle note is the one the pane shows, so its label waits for a hover, and its lines stay clear.
  const showLabel = (n: GraphNode) => hovered() === n.id || (n.id !== centre() && !!layout()?.labelled);

  /** The line between two nodes, cut short at their edges so arrowheads meet the circles. */
  function segment(l: Line) {
    const a = drawnAt(l.a);
    const b = drawnAt(l.b);
    // The middle note's ring sits 4 outside its circle; arrows stop clear of it.
    const edge = (id: string) => (layout()?.nodes.get(id)?.r ?? NODE_R) + (id === centre() ? 5.5 : 0);
    const ra = edge(l.a);
    const rb = edge(l.b);
    if (!a || !b) return null;
    const d = Math.hypot(b.x - a.x, b.y - a.y) || 1;
    const ux = (b.x - a.x) / d;
    const uy = (b.y - a.y) / d;
    const sa = ra + (l.ba ? 3 : 1);
    const sb = rb + (l.ab ? 3 : 1);
    return { x1: a.x + ux * sa, y1: a.y + uy * sa, x2: b.x - ux * sb, y2: b.y - uy * sb };
  }

  const title = (id: string) => nodes().find((n) => n.id === id)?.title || "Untitled";
  const lineTip = (l: Line) => {
    const arrow = l.ab && l.ba ? "↔" : l.ab ? "→" : "←";
    return `${title(l.a)} ${arrow} ${title(l.b)}: ${l.via.join(", ")}`;
  };

  return (
    <div class="graph-view" ref={box} data-testid="note-graph" data-center={centre()}>
      <Show when={error()}>
        <p class="graph-message error">{error()}</p>
      </Show>
      <Show when={graph() && nodes().length <= 1 && !error()}>
        <p class="graph-message muted" data-testid="graph-empty">
          No links yet.
        </p>
      </Show>
      <Show when={nodes().length > 1}>
        <svg
          ref={svg}
          class="graph-svg"
          width={width()}
          height={GRAPH_HEIGHT}
          role="group"
          aria-label={`Links of ${title(centre())}`}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        >
          <defs>
            <For each={["out", "in", "both", "side"] as const}>
              {(kind) => (
                <marker
                  id={`${markers}-${kind}`}
                  class={`graph-arrow graph-arrow-${kind}`}
                  viewBox="0 0 10 10"
                  refX="9"
                  refY="5"
                  markerUnits="userSpaceOnUse"
                  markerWidth={kind === "side" ? 6 : 8}
                  markerHeight={kind === "side" ? 6 : 8}
                  orient="auto-start-reverse"
                >
                  <path d="M0,1 L10,5 L0,9 z" />
                </marker>
              )}
            </For>
          </defs>
          <g
            class="graph-world"
            transform={`translate(${width() / 2 + drawnView().x} ${GRAPH_HEIGHT / 2 + drawnView().y}) scale(${scale()})`}
          >
            <g class="graph-lines">
              <For each={lines()}>
                {(l) => (
                  <Show when={segment(l)}>
                    {(s) => (
                      <line
                        class={`graph-line graph-line-${l.kind}`}
                        classList={{ dim: !lineLit(l), hl: !!hovered() && lineLit(l) }}
                        data-from={l.a}
                        data-to={l.b}
                        data-direction={l.kind}
                        data-forward={l.ab ? "" : undefined}
                        data-backward={l.ba ? "" : undefined}
                        x1={s().x1}
                        y1={s().y1}
                        x2={s().x2}
                        y2={s().y2}
                        stroke-width={1.4 / Math.max(0.6, scale())}
                        marker-end={l.ab ? `url(#${markers}-${l.kind})` : undefined}
                        marker-start={l.ba ? `url(#${markers}-${l.kind})` : undefined}
                      >
                        <title>{lineTip(l)}</title>
                      </line>
                    )}
                  </Show>
                )}
              </For>
            </g>
            <For each={nodes()}>
              {(n) => {
                const p = () => drawnAt(n.id);
                const r = () => layout()?.nodes.get(n.id)?.r ?? NODE_R;
                const isCentre = () => n.id === centre();
                const body = () => (
                  <g class="graph-node-body">
                    <title>{`${n.title.trim() || "Untitled"} (${typeLabel(n.type)})`}</title>
                    <Show when={isCentre()}>
                      <circle class="graph-ring" r={r() + 4} />
                    </Show>
                    <circle class="graph-dot" r={r()} style={{ fill: nodeColour(n.type) }} />
                    <Show when={showIcons()}>
                      <g class="graph-icon" transform={`translate(${-r() * 0.62} ${-r() * 0.62})`}>
                        <TypeIcon name={typeDef(n.type)?.icon} size={r() * 1.24} />
                      </g>
                    </Show>
                    <Show when={showLabel(n)}>
                      <text
                        class="graph-label"
                        y={r() + (isCentre() ? 13 : 12) / scale()}
                        text-anchor="middle"
                        stroke-width={3 / scale()}
                        font-size={String((isCentre() ? 11.5 : 10.5) / scale())}
                      >
                        {shortLabel(n.title)}
                      </text>
                    </Show>
                  </g>
                );
                return (
                  <Show when={p()}>
                    {(pos) => (
                      <g
                        class="graph-node-wrap"
                        classList={{ dim: !lit(n.id) }}
                        transform={`translate(${pos().x} ${pos().y})`}
                        opacity={appearing(n.id) ? e() : 1}
                        onPointerEnter={() => setHovered(n.id)}
                        onPointerLeave={() => setHovered((h) => (h === n.id ? null : h))}
                      >
                        <a
                          class="graph-node"
                          classList={{ "graph-centre": isCentre() }}
                          href={isCentre() ? undefined : `/notes/${n.id}`}
                          role={isCentre() ? "img" : undefined}
                          data-node-id={n.id}
                          data-direction={n.direction}
                          aria-label={nodeName(n)}
                          onFocus={() => setHovered(n.id)}
                          onBlur={() => setHovered((h) => (h === n.id ? null : h))}
                        >
                          {body()}
                        </a>
                      </g>
                    )}
                  </Show>
                );
              }}
            </For>
          </g>
        </svg>
        <Show when={(graph()?.truncated ?? 0) > 0}>
          <span class="graph-more" title="The most recently updated links show; the rest are left out">
            +{graph()!.truncated} more
          </span>
        </Show>
        <button
          type="button"
          class="graph-reset"
          aria-label="Reset graph view"
          title="Reset view"
          onClick={() => {
            viewFrom = shownView;
            from = new Map(shown);
            fitFrom = shownFit;
            tween(HOME);
          }}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 12a9 9 0 1 0 3-6.7" />
            <path d="M3 4v5h5" />
          </svg>
        </button>
      </Show>
    </div>
  );
}
