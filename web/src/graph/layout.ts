// Where each note of a graph is drawn: the note in the middle and its neighbours in rings around
// it. Neighbours are sorted (links out, both ways, links in, then type and title) and dealt to
// slots taken in angle order across every ring, so the notes it links to sit on the right, those
// both ways at the bottom and those linking in on the left, however many rings there are. A ring
// holds no more neighbours than fit at `SPACING` apart, and rings are `RING_GAP` apart, so no two
// nodes overlap. The same graph always gets the same layout.

import type { GraphDirection, GraphEdge, GraphNode } from "../api";

/** A neighbour's radius, and the note's in the middle. */
export const NODE_R = 9;
export const CENTER_R = 15;
/** The first ring's least radius, and the distance between rings. */
export const FIRST_RING = 84;
export const RING_GAP = 58;
/** The least distance along a ring between two neighbours' centres. */
export const SPACING = 50;
/** The least distance along a ring when every neighbour shows its label. */
export const LABEL_SPACING = 74;
/** Up to this many neighbours, every label shows; beyond it, labels show on hover and focus. */
export const LABELLED = 12;

export interface Placed {
  id: string;
  x: number;
  y: number;
  r: number;
  /** 0 for the note in the middle, then 1 for the first ring and so on. */
  ring: number;
}

export interface Layout {
  nodes: Map<string, Placed>;
  /** How far the nodes reach from the middle, across and down, counting their radius. */
  width: number;
  height: number;
  /** Whether every neighbour shows its label. */
  labelled: boolean;
}

const RANK: Record<GraphDirection, number> = { center: 0, out: 1, both: 2, in: 3 };

/** The neighbours in drawing order: around the ring from the top, clockwise. */
export function sortNeighbours(nodes: readonly GraphNode[]): GraphNode[] {
  return [...nodes].sort(
    (a, b) =>
      RANK[a.direction] - RANK[b.direction] ||
      a.type.localeCompare(b.type) ||
      a.title.toLowerCase().localeCompare(b.title.toLowerCase()) ||
      a.id.localeCompare(b.id),
  );
}

/** How many neighbours ring `k` (0 the first) holds at radius `r`, `gap` apart. */
function capacity(r: number, gap: number): number {
  return Math.max(1, Math.floor((2 * Math.PI * r) / gap));
}

/** The layout of the note `center` and its neighbours. */
export function layoutGraph(center: string, neighbours: readonly GraphNode[]): Layout {
  const sorted = sortNeighbours(neighbours.filter((n) => n.id !== center));
  const n = sorted.length;
  const labelled = n <= LABELLED;
  const gap = labelled ? LABEL_SPACING : SPACING;
  const nodes = new Map<string, Placed>();
  nodes.set(center, { id: center, x: 0, y: 0, r: CENTER_R, ring: 0 });

  // The rings: the first grows to hold every neighbour when they fit in one ring of labels.
  const radii: number[] = [];
  const caps: number[] = [];
  let room = 0;
  const first = labelled ? Math.max(FIRST_RING, (n * gap) / (2 * Math.PI)) : FIRST_RING;
  while (room < n) {
    const r = first + radii.length * RING_GAP;
    radii.push(r);
    caps.push(capacity(r, gap));
    room += caps[caps.length - 1]!;
  }
  // Each ring's share, in proportion to what it holds, never over it.
  const counts = caps.map((c) => Math.floor((n * c) / room));
  let left = n - counts.reduce((a, b) => a + b, 0);
  for (let k = 0; left > 0; k = (k + 1) % caps.length) {
    if (counts[k]! < caps[k]!) {
      counts[k]!++;
      left--;
    }
  }

  // Every slot of every ring, in angle order from the top, clockwise.
  const slots: { ring: number; angle: number }[] = [];
  counts.forEach((count, k) => {
    for (let j = 0; j < count; j++) {
      // Alternate rings start half a step round, so their nodes do not line up behind each other.
      const step = (2 * Math.PI) / count;
      slots.push({ ring: k, angle: step * (j + (k % 2 === 0 ? 0.5 : 1)) });
    }
  });
  slots.sort((a, b) => a.angle - b.angle || a.ring - b.ring);
  if (n === 1) {
    // A lone neighbour sits beside the note: right when the note links to it, left when it links in.
    slots[0]!.angle = sorted[0]!.direction === "in" ? 1.5 * Math.PI : 0.5 * Math.PI;
  }

  let width = CENTER_R;
  let height = CENTER_R;
  sorted.forEach((node, i) => {
    const slot = slots[i]!;
    const r = radii[slot.ring]!;
    // Angle 0 is straight up; positive is clockwise.
    const x = round(r * Math.sin(slot.angle));
    const y = round(-r * Math.cos(slot.angle));
    nodes.set(node.id, { id: node.id, x, y, r: NODE_R, ring: slot.ring + 1 });
    width = Math.max(width, Math.abs(x) + NODE_R);
    height = Math.max(height, Math.abs(y) + NODE_R);
  });
  return { nodes, width, height, labelled };
}

function round(v: number): number {
  return Math.round(v * 100) / 100;
}

/** One line between two nodes: the links between them either way, merged. */
export interface Line {
  /** The two notes, in the order the first link found ran. */
  a: string;
  b: string;
  /** Whether a links to b, and b to a. */
  ab: boolean;
  ba: boolean;
  /** `out` from the middle note, `in` to it, `both`, or `side` between two neighbours. */
  kind: "out" | "in" | "both" | "side";
  /** Each link as `field` or `field (mention)`, for the line's tooltip. */
  via: string[];
}

/** The graph's edges as lines: one per pair of notes, with the direction of each link. */
export function linesOf(center: string, edges: readonly GraphEdge[]): Line[] {
  const byPair = new Map<string, Line>();
  for (const e of edges) {
    if (e.from === e.to) continue;
    const key = e.from < e.to ? `${e.from} ${e.to}` : `${e.to} ${e.from}`;
    let line = byPair.get(key);
    if (!line) {
      line = { a: e.from, b: e.to, ab: false, ba: false, kind: "side", via: [] };
      byPair.set(key, line);
    }
    if (e.from === line.a) line.ab = true;
    else line.ba = true;
    const via = e.via === "mention" ? `${e.field} (mention)` : e.field;
    if (!line.via.includes(via)) line.via.push(via);
  }
  const lines = [...byPair.values()];
  for (const l of lines) {
    // Lines at the middle note run from it, so `ab` is always the link out.
    if (l.b === center) {
      [l.a, l.b, l.ab, l.ba] = [l.b, l.a, l.ba, l.ab];
    }
    if (l.a === center) l.kind = l.ab && l.ba ? "both" : l.ab ? "out" : "in";
  }
  return lines;
}
