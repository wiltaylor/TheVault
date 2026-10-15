// A canvas (docs/vault-format.md, "Canvas", D15) read from its block, and the edits the canvas
// editor makes to it. Edits work on the block itself, so items this client does not edit are kept
// as they were. A node is a card, which holds content blocks as a content field does, or a note.
// The server reads canvases of the older node kinds as cards, so this client only meets cards.

import type { Block, Json } from "../api";

export interface Viewport {
  /** The canvas point at the top-left of the view. */
  x: number;
  y: number;
  /** Scale factor; 1 is 100%. */
  zoom: number;
}

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface CanvasGroup extends Box {
  id: string;
  label: string;
  colour: string | null;
}

export type NodeKind = "card" | "note";

export interface CanvasNode extends Box {
  id: string;
  kind: NodeKind | string;
  group: string | null;
  colour: string | null;
  /** The node's block, for the items its kind adds. A card's content blocks are its blocks. */
  block: Block;
}

export type Side = "top" | "right" | "bottom" | "left";

/** Where an edge draws arrowheads: `start` is at the `from` node, `end` at the `to` node. */
export type Arrow = "none" | "start" | "end" | "both";
export const ARROWS: Arrow[] = ["none", "start", "end", "both"];

export interface CanvasEdge {
  id: string;
  from: string;
  to: string;
  fromSide: Side | null;
  toSide: Side | null;
  label: string;
  colour: string | null;
  arrow: Arrow;
}

export interface Canvas {
  /** The canvas's height on the page in screen pixels, or null for the default. */
  height: number | null;
  viewport: Viewport;
  groups: CanvasGroup[];
  nodes: CanvasNode[];
  edges: CanvasEdge[];
}

export const SIDES: Side[] = ["top", "right", "bottom", "left"];

export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 4;

/** The size of a node made from scratch, in canvas units. */
export const NEW_NODE = { width: 240, height: 140 };
export const NEW_GROUP = { width: 560, height: 360 };

/** The smallest a node or group can be resized to. */
export const MIN_SIZE = 40;

/** A canvas's height on the page, in screen pixels: the default and the bounds the server checks. */
export const CANVAS_HEIGHT = { initial: 480, min: 160, max: 4000 };

export function clampHeight(h: number): number {
  return Math.round(Math.min(CANVAS_HEIGHT.max, Math.max(CANVAS_HEIGHT.min, h)));
}

const num = (v: Json | undefined, fallback: number) => (typeof v === "number" && Number.isFinite(v) ? v : fallback);
const str = (v: Json | undefined): string | null => (typeof v === "string" ? v : null);

/** Coordinates as stored: two decimal places. */
export function round(v: number): number {
  return Math.round(v * 100) / 100;
}

export function clampZoom(z: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));
}

function boxOf(b: Block, w: number, h: number): Box {
  const f = b.fields ?? {};
  return {
    x: num(f.x, 0),
    y: num(f.y, 0),
    width: Math.max(1, num(f.width, w)),
    height: Math.max(1, num(f.height, h)),
  };
}

/** The canvas a `canvas` block holds. Missing items read as their defaults. */
export function canvasOf(block: Block | undefined): Canvas {
  const blocks = block?.blocks ?? [];
  const vp = blocks.find((b) => b.kind === "viewport")?.fields ?? {};
  const zoom = num(vp.zoom, 1);
  const id = (b: Block) => b.labels?.[0] ?? "";
  const h = block?.fields?.height;
  return {
    height: typeof h === "number" && Number.isFinite(h) ? clampHeight(h) : null,
    viewport: { x: num(vp.x, 0), y: num(vp.y, 0), zoom: zoom > 0 ? zoom : 1 },
    groups: blocks
      .filter((b) => b.kind === "group")
      .map((b) => ({
        id: id(b),
        label: str(b.fields?.label) ?? "",
        colour: str(b.fields?.colour),
        ...boxOf(b, NEW_GROUP.width, NEW_GROUP.height),
      })),
    nodes: blocks
      .filter((b) => b.kind === "node")
      .map((b) => ({
        id: id(b),
        kind: str(b.fields?.kind) ?? "card",
        group: str(b.fields?.group),
        colour: str(b.fields?.colour),
        block: b,
        ...boxOf(b, NEW_NODE.width, NEW_NODE.height),
      })),
    edges: blocks
      .filter((b) => b.kind === "edge")
      .map((b) => {
        const f = b.fields ?? {};
        const side = (v: Json | undefined) => (SIDES.includes(v as Side) ? (v as Side) : null);
        const arrow = ARROWS.includes(f.arrow as Arrow) ? (f.arrow as Arrow) : "end";
        return {
          id: id(b),
          from: str(f.from) ?? "",
          to: str(f.to) ?? "",
          fromSide: side(f.from_side),
          toSide: side(f.to_side),
          label: str(f.label) ?? "",
          colour: str(f.colour),
          arrow,
        };
      }),
  };
}

/** A new canvas: an empty view at the origin, at 100%. */
export function newCanvas(): Block {
  return { kind: "canvas", blocks: [{ kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } }] };
}

/** `block`, or a new canvas when there is none yet. */
function base(block: Block | undefined): Block {
  return block ?? newCanvas();
}

function mapItems(block: Block, edit: (b: Block) => Block | null): Block {
  return {
    ...block,
    blocks: (block.blocks ?? []).map(edit).filter((b): b is Block => b !== null),
  };
}

/** `block` with its viewport set. The viewport stays the first item. */
export function withViewport(block: Block | undefined, vp: Viewport): Block {
  const b = base(block);
  const fields = { x: round(vp.x), y: round(vp.y), zoom: Math.round(clampZoom(vp.zoom) * 1000) / 1000 };
  const rest = (b.blocks ?? []).filter((x) => x.kind !== "viewport");
  return { ...b, blocks: [{ kind: "viewport", fields }, ...rest] };
}

/** `block` with its height set, within the bounds; the default height leaves no item. */
export function withHeight(block: Block | undefined, height: number): Block {
  const b = base(block);
  const fields = { ...b.fields };
  const h = clampHeight(height);
  if (h === CANVAS_HEIGHT.initial) delete fields.height;
  else fields.height = h;
  const next: Block = { ...b, fields };
  if (Object.keys(fields).length === 0) delete next.fields;
  return next;
}

const ID_ALPHABET = "0123456789abcdefghijklmnopqrstuvwxyz";
const ID_SUFFIX_LENGTH = 5;

/**
 * An id no group, node or edge of `block` has yet: `prefix` and a short random suffix, such as
 * `card_k3f9x`. The suffix keeps ids made on two devices offline from clashing when they sync
 * (D30); ids already in the canvas are never changed.
 */
export function freeId(block: Block | undefined, prefix: string): string {
  const taken = new Set((block?.blocks ?? []).map((b) => b.labels?.[0]));
  for (;;) {
    let suffix = "";
    for (let i = 0; i < ID_SUFFIX_LENGTH; i++) suffix += ID_ALPHABET[Math.floor(Math.random() * ID_ALPHABET.length)];
    const id = `${prefix}_${suffix}`;
    if (!taken.has(id)) return id;
  }
}

/**
 * `block` with a new node of `kind` whose box starts at (x, y), and the node's id. `items` are
 * the items the node kind adds, such as a note node's `note`; `blocks` are a card's blocks.
 */
export function addNode(
  block: Block | undefined,
  kind: NodeKind,
  at: { x: number; y: number },
  items: Record<string, Json> = {},
  blocks: Block[] = [],
): { block: Block; id: string } {
  const b = base(block);
  const id = freeId(b, kind);
  const size = NEW_NODE;
  const node: Block = {
    kind: "node",
    labels: [id],
    fields: { kind, x: round(at.x), y: round(at.y), width: size.width, height: size.height, ...items },
  };
  if (blocks.length > 0) node.blocks = blocks;
  // A node drawn over a group's frame belongs to it.
  const group = canvasOf(b).groups.find((g) => contains(g, at.x, at.y));
  if (group) node.fields!.group = group.id;
  return { block: { ...b, blocks: [...(b.blocks ?? []), node] }, id };
}

/** `block` with a new group whose frame starts at (x, y), and the group's id. */
export function addGroup(block: Block | undefined, at: { x: number; y: number }, label: string): { block: Block; id: string } {
  const b = base(block);
  const id = freeId(b, "group");
  const group: Block = {
    kind: "group",
    labels: [id],
    fields: { label, x: round(at.x), y: round(at.y), width: NEW_GROUP.width, height: NEW_GROUP.height },
  };
  return { block: { ...b, blocks: [...(b.blocks ?? []), group] }, id };
}

/** `block` with an edge from node `from` to node `to`, or unchanged when one joins them already. */
export function addEdge(block: Block, from: string, to: string): { block: Block; id: string | null } {
  if (from === to) return { block, id: null };
  const c = canvasOf(block);
  if (c.edges.some((e) => e.from === from && e.to === to)) return { block, id: null };
  const id = freeId(block, `${from}_to_${to}`);
  const edge: Block = { kind: "edge", labels: [id], fields: { from, to } };
  return { block: { ...block, blocks: [...(block.blocks ?? []), edge] }, id };
}

/** `block` with item `name` of the group, node or edge `id` set, or removed when undefined. */
export function setItem(block: Block, id: string, name: string, value: Json | undefined): Block {
  return mapItems(block, (b) => {
    if (b.kind === "viewport" || b.labels?.[0] !== id) return b;
    const fields = { ...b.fields };
    if (value === undefined) delete fields[name];
    else fields[name] = value;
    return { ...b, fields };
  });
}

/** `block` with card `id` holding `blocks`, its content blocks, in order. */
export function setNodeBlocks(block: Block, id: string, blocks: Block[]): Block {
  return mapItems(block, (b) => {
    if (b.kind !== "node" || b.labels?.[0] !== id) return b;
    const next: Block = { ...b, blocks };
    if (blocks.length === 0) delete next.blocks;
    return next;
  });
}

/** The arrow item to store for `arrow`: none when it is the default, `end`. */
export function arrowItem(arrow: Arrow): Arrow | undefined {
  return arrow === "end" ? undefined : arrow;
}

/** Which ends of an edge draw an arrowhead. */
export function arrowEnds(arrow: Arrow): { start: boolean; end: boolean } {
  return { start: arrow === "start" || arrow === "both", end: arrow === "end" || arrow === "both" };
}

/**
 * The triangle of an arrowhead whose tip is at `tip`, pointing along the run from `from` to
 * `tip`, as SVG polygon points. `length` and `width` are in canvas units.
 */
export function arrowHead(from: Point, tip: Point, length = 10, width = 9): string {
  const dx = tip.x - from.x;
  const dy = tip.y - from.y;
  const l = Math.hypot(dx, dy) || 1;
  const ux = dx / l;
  const uy = dy / l;
  const bx = tip.x - ux * length;
  const by = tip.y - uy * length;
  const f = (n: number) => Math.round(n * 100) / 100;
  const pts = [
    [tip.x, tip.y],
    [bx - uy * (width / 2), by + ux * (width / 2)],
    [bx + uy * (width / 2), by - ux * (width / 2)],
  ];
  return pts.map(([x, y]) => `${f(x!)},${f(y!)}`).join(" ");
}

/**
 * `points` with its first and last point pulled back `by` along their segments where `start` or
 * `end` is true, so a line stops short of the tip of an arrowhead drawn there.
 */
export function trimRoute(points: Point[], start: boolean, end: boolean, by = 4): Point[] {
  if (points.length < 2) return points;
  const pull = (tip: Point, from: Point): Point => {
    const l = Math.hypot(tip.x - from.x, tip.y - from.y);
    if (l <= by) return tip;
    return { x: tip.x - ((tip.x - from.x) / l) * by, y: tip.y - ((tip.y - from.y) / l) * by };
  };
  const out = [...points];
  if (start) out[0] = pull(points[0]!, points[1]!);
  if (end) out[out.length - 1] = pull(points[points.length - 1]!, points[points.length - 2]!);
  return out;
}

/**
 * The direction a route runs, from its `from` end towards its `to` end, in degrees clockwise from
 * pointing right, along the main axis between its two ends: 0, 90, 180 or 270. The jogs an elbow
 * route makes on the way do not count.
 */
export function routeDirection(points: Point[]): number {
  const a = points[0];
  const b = points.at(-1);
  if (!a || !b) return 0;
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  if (Math.abs(dx) >= Math.abs(dy)) return dx < 0 ? 180 : 0;
  return dy < 0 ? 270 : 90;
}

function shift(b: Block, dx: number, dy: number): Block {
  const f = b.fields ?? {};
  return { ...b, fields: { ...f, x: round(num(f.x, 0) + dx), y: round(num(f.y, 0) + dy) } };
}

/**
 * `block` with group or node `id` moved by (dx, dy). Moving a group moves its nodes. A node
 * dropped with its top-left corner on a group's frame joins that group, and leaves it when
 * dropped outside every group.
 */
export function moveItem(block: Block, id: string, dx: number, dy: number): Block {
  const c = canvasOf(block);
  const isGroup = c.groups.some((g) => g.id === id);
  const moved = mapItems(block, (b) => {
    const label = b.labels?.[0];
    if (b.kind === "group" && label === id) return shift(b, dx, dy);
    if (b.kind === "node" && (label === id || (isGroup && b.fields?.group === id))) return shift(b, dx, dy);
    return b;
  });
  if (isGroup) return moved;
  const node = canvasOf(moved).nodes.find((n) => n.id === id);
  if (!node) return moved;
  const inside = c.groups.find((g) => contains(g, node.x, node.y));
  return setItem(moved, id, "group", inside?.id);
}

/** `block` with group or node `id` given a new size, no smaller than MIN_SIZE. */
export function resizeItem(block: Block, id: string, width: number, height: number): Block {
  return mapItems(block, (b) => {
    if ((b.kind !== "node" && b.kind !== "group") || b.labels?.[0] !== id) return b;
    return {
      ...b,
      fields: { ...b.fields, width: round(Math.max(MIN_SIZE, width)), height: round(Math.max(MIN_SIZE, height)) },
    };
  });
}

/**
 * `block` without group, node or edge `id`. A node takes its edges with it; a group leaves its
 * nodes where they are, outside any group.
 */
export function removeItem(block: Block, id: string): Block {
  return mapItems(block, (b) => {
    if (b.kind === "viewport") return b;
    if (b.labels?.[0] === id) return null;
    if (b.kind === "edge" && (b.fields?.from === id || b.fields?.to === id)) return null;
    if (b.kind === "node" && b.fields?.group === id) {
      const fields = { ...b.fields };
      delete fields.group;
      return { ...b, fields };
    }
    return b;
  });
}

export function contains(box: Box, x: number, y: number): boolean {
  return x >= box.x && x <= box.x + box.width && y >= box.y && y <= box.y + box.height;
}

/** The viewport after zooming by `factor` about the screen point (sx, sy), which stays put. */
export function zoomAbout(vp: Viewport, factor: number, sx: number, sy: number): Viewport {
  const zoom = clampZoom(vp.zoom * factor);
  // The canvas point under (sx, sy) before and after must be the same.
  const cx = vp.x + sx / vp.zoom;
  const cy = vp.y + sy / vp.zoom;
  return { x: cx - sx / zoom, y: cy - sy / zoom, zoom };
}

/** The canvas point under screen point (sx, sy), relative to the view's top-left. */
export function toCanvas(vp: Viewport, sx: number, sy: number): { x: number; y: number } {
  return { x: vp.x + sx / vp.zoom, y: vp.y + sy / vp.zoom };
}

export interface Point {
  x: number;
  y: number;
}

/** The port in the middle of side `side` of a box, where an edge leaves or enters it. */
export function portPoint(b: Box, side: Side): Point {
  return anchor(b, side);
}

function anchor(b: Box, side: Side): { x: number; y: number } {
  switch (side) {
    case "top":
      return { x: b.x + b.width / 2, y: b.y };
    case "bottom":
      return { x: b.x + b.width / 2, y: b.y + b.height };
    case "left":
      return { x: b.x, y: b.y + b.height / 2 };
    case "right":
      return { x: b.x + b.width, y: b.y + b.height / 2 };
  }
}

/** The side of `from` that faces `to` most directly. */
export function nearestSide(from: Box, to: Box): Side {
  const dx = to.x + to.width / 2 - (from.x + from.width / 2);
  const dy = to.y + to.height / 2 - (from.y + from.height / 2);
  if (Math.abs(dx) >= Math.abs(dy)) return dx >= 0 ? "right" : "left";
  return dy >= 0 ? "bottom" : "top";
}

/** The ends of an edge between two boxes: the named sides, else the sides facing each other. */
export function edgeEnds(
  from: Box,
  to: Box,
  fromSide: Side | null,
  toSide: Side | null,
): { a: { x: number; y: number }; b: { x: number; y: number } } {
  return {
    a: anchor(from, fromSide ?? nearestSide(from, to)),
    b: anchor(to, toSide ?? nearestSide(to, from)),
  };
}

/** The outward direction of each side. */
const NORMAL: Record<Side, Point> = {
  top: { x: 0, y: -1 },
  right: { x: 1, y: 0 },
  bottom: { x: 0, y: 1 },
  left: { x: -1, y: 0 },
};

const horizontal = (s: Side) => s === "left" || s === "right";

/**
 * The pair of ports, one side of `from` and one of `to`, an edge between the two boxes should
 * join: the closest pair whose sides both face the other port. A side facing away would make the
 * edge double back round its own box, so such pairs only win when nothing faces.
 */
export function closestSides(from: Box, to: Box): { fromSide: Side; toSide: Side } {
  let best = { fromSide: "right" as Side, toSide: "left" as Side };
  let bestScore = Number.POSITIVE_INFINITY;
  for (const fs of SIDES) {
    for (const ts of SIDES) {
      const a = anchor(from, fs);
      const b = anchor(to, ts);
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      let score = Math.hypot(dx, dy);
      // A small bias for sides that face along the line between the ports.
      const faceA = dx * NORMAL[fs].x + dy * NORMAL[fs].y;
      const faceB = -dx * NORMAL[ts].x - dy * NORMAL[ts].y;
      if (faceA < 0) score += 100000;
      if (faceB < 0) score += 100000;
      if (score < bestScore - 1e-9) {
        bestScore = score;
        best = { fromSide: fs, toSide: ts };
      }
    }
  }
  return best;
}

/** How far an edge runs straight out of a port before it turns. */
export const STUB = 20;

/** `points` without repeats and without middle points on a straight run. */
function simplify(points: Point[]): Point[] {
  const out: Point[] = [];
  for (const p of points) {
    const last = out.at(-1);
    if (last && Math.abs(last.x - p.x) < 1e-6 && Math.abs(last.y - p.y) < 1e-6) continue;
    const prev = out.at(-2);
    if (last && prev) {
      const straight =
        (Math.abs(prev.x - last.x) < 1e-6 && Math.abs(last.x - p.x) < 1e-6) ||
        (Math.abs(prev.y - last.y) < 1e-6 && Math.abs(last.y - p.y) < 1e-6);
      if (straight) out.pop();
    }
    out.push(p);
  }
  return out;
}

/**
 * An orthogonal route from port `a` on side `sa` to port `b` on side `sb`: it leaves `a` and
 * enters `b` at right angles to their sides, and every segment is horizontal or vertical.
 */
export function elbowRoute(a: Point, sa: Side, b: Point, sb: Side, stub = STUB): Point[] {
  const p = { x: a.x + NORMAL[sa].x * stub, y: a.y + NORMAL[sa].y * stub };
  const q = { x: b.x + NORMAL[sb].x * stub, y: b.y + NORMAL[sb].y * stub };
  let middle: Point[];
  if (horizontal(sa) && horizontal(sb)) {
    const facing = (sa === "right" && sb === "left" && p.x <= q.x) || (sa === "left" && sb === "right" && p.x >= q.x);
    if (facing) {
      const mx = (p.x + q.x) / 2;
      middle = [{ x: mx, y: p.y }, { x: mx, y: q.y }];
    } else if (sa === sb) {
      const x = sa === "right" ? Math.max(p.x, q.x) : Math.min(p.x, q.x);
      middle = [{ x, y: p.y }, { x, y: q.y }];
    } else {
      const my = (p.y + q.y) / 2;
      middle = [{ x: p.x, y: my }, { x: q.x, y: my }];
    }
  } else if (!horizontal(sa) && !horizontal(sb)) {
    const facing = (sa === "bottom" && sb === "top" && p.y <= q.y) || (sa === "top" && sb === "bottom" && p.y >= q.y);
    if (facing) {
      const my = (p.y + q.y) / 2;
      middle = [{ x: p.x, y: my }, { x: q.x, y: my }];
    } else if (sa === sb) {
      const y = sa === "bottom" ? Math.max(p.y, q.y) : Math.min(p.y, q.y);
      middle = [{ x: p.x, y }, { x: q.x, y }];
    } else {
      const mx = (p.x + q.x) / 2;
      middle = [{ x: mx, y: p.y }, { x: mx, y: q.y }];
    }
  } else if (horizontal(sa)) {
    middle = [{ x: q.x, y: p.y }];
  } else {
    middle = [{ x: p.x, y: q.y }];
  }
  return simplify([a, p, ...middle, q, b]);
}

/** An SVG path along `points` with each corner rounded to radius `r` at most. */
export function roundedPath(points: Point[], r = 8): string {
  if (points.length === 0) return "";
  const f = (n: number) => Math.round(n * 100) / 100;
  let d = `M${f(points[0]!.x)},${f(points[0]!.y)}`;
  for (let i = 1; i < points.length - 1; i++) {
    const prev = points[i - 1]!;
    const at = points[i]!;
    const next = points[i + 1]!;
    const lin = Math.hypot(at.x - prev.x, at.y - prev.y);
    const lout = Math.hypot(next.x - at.x, next.y - at.y);
    const k = Math.min(r, lin / 2, lout / 2);
    const enter = { x: at.x - ((at.x - prev.x) / (lin || 1)) * k, y: at.y - ((at.y - prev.y) / (lin || 1)) * k };
    const leave = { x: at.x + ((next.x - at.x) / (lout || 1)) * k, y: at.y + ((next.y - at.y) / (lout || 1)) * k };
    d += ` L${f(enter.x)},${f(enter.y)} Q${f(at.x)},${f(at.y)} ${f(leave.x)},${f(leave.y)}`;
  }
  const last = points[points.length - 1]!;
  return `${d} L${f(last.x)},${f(last.y)}`;
}

/** The point halfway along a route, where an edge's label goes. */
export function midpoint(points: Point[]): Point {
  const lengths = points.slice(1).map((p, i) => Math.hypot(p.x - points[i]!.x, p.y - points[i]!.y));
  let half = lengths.reduce((a, b) => a + b, 0) / 2;
  for (let i = 0; i < lengths.length; i++) {
    const l = lengths[i]!;
    if (half <= l && l > 0) {
      const a = points[i]!;
      const b = points[i + 1]!;
      return { x: a.x + ((b.x - a.x) * half) / l, y: a.y + ((b.y - a.y) * half) / l };
    }
    half -= l;
  }
  return points[0] ?? { x: 0, y: 0 };
}

/** The route an edge between two boxes is drawn along, between its sides or the closest pair. */
export function edgeRoute(from: Box, to: Box, fromSide: Side | null, toSide: Side | null): Point[] {
  const sides = fromSide && toSide ? { fromSide, toSide } : closestSides(from, to);
  return elbowRoute(anchor(from, sides.fromSide), sides.fromSide, anchor(to, sides.toSide), sides.toSide);
}

/**
 * `block` with every edge's `from_side` and `to_side` set to the closest port pair of its two
 * nodes, so the file says what is drawn. Run after anything that moves, resizes or joins nodes.
 */
export function resnapEdges(block: Block): Block {
  const c = canvasOf(block);
  const boxes = new Map(c.nodes.map((n) => [n.id, n]));
  let changed = false;
  const blocks = (block.blocks ?? []).map((b) => {
    if (b.kind !== "edge") return b;
    const from = boxes.get(String(b.fields?.from ?? ""));
    const to = boxes.get(String(b.fields?.to ?? ""));
    if (!from || !to) return b;
    const { fromSide, toSide } = closestSides(from, to);
    if (b.fields?.from_side === fromSide && b.fields?.to_side === toSide) return b;
    changed = true;
    return { ...b, fields: { ...b.fields, from_side: fromSide, to_side: toSide } };
  });
  return changed ? { ...block, blocks } : block;
}

/**
 * The view that shows every node and group of `c` in a view `width` by `height` screen pixels,
 * with a margin, no closer than 100%. An empty canvas fits at the origin at 100%.
 */
export function fitView(c: Canvas, width: number, height: number, margin = 40): Viewport {
  const items: Box[] = [...c.nodes, ...c.groups];
  if (items.length === 0 || width <= 0 || height <= 0) return { x: 0, y: 0, zoom: 1 };
  const x0 = Math.min(...items.map((b) => b.x));
  const y0 = Math.min(...items.map((b) => b.y));
  const x1 = Math.max(...items.map((b) => b.x + b.width));
  const y1 = Math.max(...items.map((b) => b.y + b.height));
  const bw = Math.max(1, x1 - x0);
  const bh = Math.max(1, y1 - y0);
  const zoom = clampZoom(Math.min(1, (width - 2 * margin) / bw, (height - 2 * margin) / bh));
  return { x: x0 - (width / zoom - bw) / 2, y: y0 - (height / zoom - bh) / 2, zoom };
}
