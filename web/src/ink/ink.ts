// An ink drawing (docs/vault-format.md, "Ink", D18, D29) to and from its block, and the outline
// perfect-freehand draws for each stroke. The renderer and the editor share this, so a stroke
// looks the same while it is drawn, once saved, and after a reload.

import { getStroke } from "perfect-freehand";
import type { Block, Json } from "../api";

export type Tool = "pen" | "highlighter";
export type Background = "blank" | "lines" | "grid" | "dots";

export interface Stroke {
  tool: Tool;
  colour: string;
  size: number;
  /** Flat triples: x, y, pressure. */
  points: number[];
}

export interface Ink {
  width: number;
  height: number;
  background: Background;
  strokes: Stroke[];
}

export const BACKGROUNDS: Background[] = ["blank", "lines", "grid", "dots"];

/** The size of a drawing made from scratch, in ink units. */
export const NEW_WIDTH = 800;
export const NEW_HEIGHT = 400;

/** The pressure stored for a device that reports none. */
export const NO_PRESSURE = 0.5;

const num = (v: Json | undefined, fallback: number) => (typeof v === "number" && Number.isFinite(v) ? v : fallback);

/** The drawing an `ink` block holds. Missing items read as their defaults. */
export function inkOf(block: Block | undefined): Ink {
  const f = block?.fields ?? {};
  const bg = f.background;
  return {
    width: Math.max(1, Math.round(num(f.width, NEW_WIDTH))),
    height: Math.max(1, Math.round(num(f.height, NEW_HEIGHT))),
    background: BACKGROUNDS.includes(bg as Background) ? (bg as Background) : "blank",
    strokes: (block?.blocks ?? [])
      .filter((b) => b.kind === "stroke")
      .map((b) => ({
        tool: b.fields?.tool === "highlighter" ? "highlighter" : "pen",
        colour: typeof b.fields?.colour === "string" ? b.fields.colour : "#1f2937",
        size: num(b.fields?.size, 4),
        points: Array.isArray(b.fields?.points) ? b.fields.points.filter((p): p is number => typeof p === "number") : [],
      })),
  };
}

const round = (v: number, places: number) => {
  const f = 10 ** places;
  return Math.round(v * f) / f;
};

/** A point as stored: coordinates to two decimal places, pressure clamped and to three. */
export function storedPoint(x: number, y: number, pressure: number): [number, number, number] {
  return [round(x, 2), round(y, 2), round(Math.min(1, Math.max(0, pressure)), 3)];
}

/**
 * `ink` as a block, keeping `base`'s kind and labels: an ink content block has none, an ink
 * field's block is labelled with the field name. `background` is left out while it is `blank`.
 */
export function inkBlock(ink: Ink, base?: Block): Block {
  const fields: Record<string, Json> = { width: ink.width, height: ink.height };
  if (ink.background !== "blank") fields.background = ink.background;
  const block: Block = {
    kind: "ink",
    fields,
    blocks: ink.strokes.map((s) => ({
      kind: "stroke",
      fields: { tool: s.tool, colour: s.colour, size: s.size, points: s.points },
    })),
  };
  return base?.labels ? { ...block, labels: base.labels } : block;
}

/** The pressure a device reported, or NO_PRESSURE when it reports none. */
export function pointerPressure(e: { pointerType: string; pressure: number }): number {
  // A mouse reports 0.5 while a button is down; touch screens without force report 0 or 1.
  if (e.pointerType === "pen") return e.pressure > 0 ? e.pressure : NO_PRESSURE;
  if (e.pointerType === "touch" && e.pressure > 0 && e.pressure < 1) return e.pressure;
  return NO_PRESSURE;
}

function triples(points: number[]): [number, number, number][] {
  const out: [number, number, number][] = [];
  for (let i = 0; i + 2 < points.length; i += 3) out.push([points[i]!, points[i + 1]!, points[i + 2]!]);
  return out;
}

/**
 * The SVG path of a stroke's filled outline. Pressure is always the stored one, never simulated,
 * so the outline depends on the file alone.
 */
export function strokePath(s: Stroke, last = true): string {
  const outline = getStroke(triples(s.points), {
    size: s.size,
    thinning: 0.6,
    smoothing: 0.5,
    streamline: 0.4,
    simulatePressure: false,
    last,
  });
  if (outline.length === 0) return "";
  const [first, ...rest] = outline;
  const f = (n: number) => round(n, 2);
  let d = `M${f(first![0])},${f(first![1])}`;
  for (const [x, y] of rest) d += `L${f(x)},${f(y)}`;
  return `${d}Z`;
}

/** Distance from p to the segment a-b. */
function toSegment(px: number, py: number, ax: number, ay: number, bx: number, by: number): number {
  const dx = bx - ax;
  const dy = by - ay;
  const len = dx * dx + dy * dy;
  const t = len === 0 ? 0 : Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / len));
  return Math.hypot(px - (ax + t * dx), py - (ay + t * dy));
}

/** True when a circle of `radius` at (x, y) touches the stroke. */
export function hits(s: Stroke, x: number, y: number, radius: number): boolean {
  const reach = radius + s.size / 2;
  const p = triples(s.points);
  if (p.length === 1) return Math.hypot(x - p[0]![0], y - p[0]![1]) <= reach;
  for (let i = 1; i < p.length; i++) {
    if (toSegment(x, y, p[i - 1]![0], p[i - 1]![1], p[i]![0], p[i]![1]) <= reach) return true;
  }
  return false;
}

/** Highlighter strokes first, then pen strokes, each in drawing order: highlighters sit behind. */
export function paintOrder(strokes: Stroke[]): { stroke: Stroke; index: number }[] {
  const all = strokes.map((stroke, index) => ({ stroke, index }));
  return [...all.filter((s) => s.stroke.tool === "highlighter"), ...all.filter((s) => s.stroke.tool === "pen")];
}
