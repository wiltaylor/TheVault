// A canvas (docs/vault-format.md, "Canvas", D15) read from its block, and the edits the canvas
// editor makes to it. Edits work on the block itself, so items and nodes this client does not
// edit, such as an ink node's strokes, are kept as they were.

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

export type NodeKind = "text" | "note" | "query" | "asset" | "link" | "ink";

export interface CanvasNode extends Box {
  id: string;
  kind: NodeKind | string;
  group: string | null;
  colour: string | null;
  /** The node's block, for the items its kind adds. */
  block: Block;
}

export type Side = "top" | "right" | "bottom" | "left";

export interface CanvasEdge {
  id: string;
  from: string;
  to: string;
  fromSide: Side | null;
  toSide: Side | null;
  label: string;
  colour: string | null;
  arrow: "none" | "end" | "both";
}

export interface Canvas {
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
export const NEW_QUERY_NODE = { width: 320, height: 360 };
export const NEW_GROUP = { width: 560, height: 360 };

/** The smallest a node or group can be resized to. */
export const MIN_SIZE = 40;

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
  return {
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
        kind: str(b.fields?.kind) ?? "text",
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
        const arrow = f.arrow === "none" || f.arrow === "both" ? f.arrow : "end";
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

/** An id no group, node or edge of `block` has yet: `prefix`, then `prefix_2` and so on. */
export function freeId(block: Block | undefined, prefix: string): string {
  const taken = new Set((block?.blocks ?? []).map((b) => b.labels?.[0]));
  if (!taken.has(prefix)) return prefix;
  let n = 2;
  while (taken.has(`${prefix}_${n}`)) n++;
  return `${prefix}_${n}`;
}

/**
 * `block` with a new node of `kind` whose box starts at (x, y), and the node's id. `items` are
 * the items the node kind adds, such as a text node's `text`.
 */
export function addNode(
  block: Block | undefined,
  kind: NodeKind,
  at: { x: number; y: number },
  items: Record<string, Json>,
): { block: Block; id: string } {
  const b = base(block);
  const id = freeId(b, kind);
  const size = kind === "query" ? NEW_QUERY_NODE : NEW_NODE;
  const node: Block = {
    kind: "node",
    labels: [id],
    fields: { kind, x: round(at.x), y: round(at.y), width: size.width, height: size.height, ...items },
  };
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
