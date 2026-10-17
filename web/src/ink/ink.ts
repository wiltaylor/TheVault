// An ink drawing (docs/vault-format.md, "Ink", D18, D29) to and from its block, the outline
// perfect-freehand draws for each stroke, and the view onto the drawing's endless plane. The
// renderer and the editor share this, so a stroke looks the same while it is drawn, once saved,
// and after a reload.

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

/** The part of the plane on show: the ink point at the view's top-left corner, and the zoom. */
export interface InkView {
  x: number;
  y: number;
  /** Screen pixels per ink unit: 1 is 100%. */
  zoom: number;
}

export interface Ink {
  /**
   * The width of the page a drawing made before the plane was endless had, in ink units, when
   * its file says. Without a saved view such a drawing opens with this width fitted to the column.
   */
  width?: number;
  /** The height of the view on the page, in CSS pixels. */
  height: number;
  background: Background;
  /** The view saved with the drawing, or null for the default view. */
  view: InkView | null;
  strokes: Stroke[];
}

export const BACKGROUNDS: Background[] = ["blank", "lines", "grid", "dots"];

/** The size of a drawing made from scratch. */
export const NEW_WIDTH = 800;
export const NEW_HEIGHT = 400;

/**
 * The drawing's height: what an omitted `height` reads as and the handle resets to, and the
 * bounds the handle drags within. The server accepts 1 to `max`, so a drawing made shorter than
 * `min` before the handle existed stays as it is until it is resized.
 */
export const INK_HEIGHT = { initial: NEW_HEIGHT, min: 80, max: 4000 };

/** The pressure stored for a device that reports none. */
export const NO_PRESSURE = 0.5;

/** The zoom bounds: 10% to 800%. */
export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 8;

export function clampZoom(z: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Number.isFinite(z) && z > 0 ? z : 1));
}

const num = (v: Json | undefined, fallback: number) => (typeof v === "number" && Number.isFinite(v) ? v : fallback);

// A stroke read from a block is kept with that block, and written back as the same block while
// it is unchanged, so an edit leaves the other strokes' objects, and their drawn paths, as they were.
const strokeOf = new WeakMap<Block, Stroke>();
const blockOf = new WeakMap<Stroke, Block>();

function readStroke(b: Block): Stroke {
  const known = strokeOf.get(b);
  if (known) return known;
  const s: Stroke = {
    tool: b.fields?.tool === "highlighter" ? "highlighter" : "pen",
    colour: typeof b.fields?.colour === "string" ? b.fields.colour : DEFAULT_INK,
    size: num(b.fields?.size, 4),
    points: Array.isArray(b.fields?.points) ? b.fields.points.filter((p): p is number => typeof p === "number") : [],
  };
  strokeOf.set(b, s);
  blockOf.set(s, b);
  return s;
}

function strokeBlock(s: Stroke): Block {
  const known = blockOf.get(s);
  if (known) return known;
  const b: Block = { kind: "stroke", fields: { tool: s.tool, colour: s.colour, size: s.size, points: s.points } };
  strokeOf.set(b, s);
  blockOf.set(s, b);
  return b;
}

/** The drawing an `ink` block holds. Missing items read as their defaults. */
export function inkOf(block: Block | undefined): Ink {
  const f = block?.fields ?? {};
  const bg = f.background;
  const vp = (block?.blocks ?? []).find((b) => b.kind === "viewport")?.fields;
  const ink: Ink = {
    height: Math.max(1, Math.round(num(f.height, NEW_HEIGHT))),
    background: BACKGROUNDS.includes(bg as Background) ? (bg as Background) : "blank",
    view: vp ? { x: num(vp.x, 0), y: num(vp.y, 0), zoom: clampZoom(num(vp.zoom, 1)) } : null,
    strokes: (block?.blocks ?? []).filter((b) => b.kind === "stroke").map(readStroke),
  };
  if (typeof f.width === "number" && Number.isFinite(f.width)) ink.width = Math.max(1, Math.round(f.width));
  return ink;
}

const round = (v: number, places: number) => {
  const f = 10 ** places;
  return Math.round(v * f) / f;
};

/** A point as stored: coordinates to two decimal places, pressure clamped and to three. */
export function storedPoint(x: number, y: number, pressure: number): [number, number, number] {
  return [round(x, 2), round(y, 2), round(Math.min(1, Math.max(0, pressure)), 3)];
}

/** A view as stored: the corner to two decimal places, the zoom clamped and to three. */
export function storedView(v: InkView): InkView {
  return { x: round(v.x, 2), y: round(v.y, 2), zoom: round(clampZoom(v.zoom), 3) };
}

/**
 * `ink` as a block, keeping `base`'s kind and labels: an ink content block has none, an ink
 * field's block is labelled with the field name. `background` is left out while it is `blank`,
 * `width` while the drawing has none, and the viewport while the drawing has no saved view; the
 * viewport goes ahead of the strokes.
 */
export function inkBlock(ink: Ink, base?: Block): Block {
  const fields: Record<string, Json> = {};
  if (ink.width !== undefined) fields.width = ink.width;
  fields.height = ink.height;
  if (ink.background !== "blank") fields.background = ink.background;
  const view = ink.view ? storedView(ink.view) : null;
  const block: Block = {
    kind: "ink",
    fields,
    blocks: [
      ...(view ? [{ kind: "viewport", fields: { x: view.x, y: view.y, zoom: view.zoom } }] : []),
      ...ink.strokes.map(strokeBlock),
    ],
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
 * so the outline depends on the file alone. A highlighter keeps one width whatever the pressure,
 * as a felt tip does.
 */
export function strokePath(s: Stroke, last = true): string {
  const outline = getStroke(triples(s.points), {
    size: s.size,
    thinning: s.tool === "highlighter" ? 0 : 0.6,
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

const paths = new WeakMap<Stroke, string>();

/** strokePath of a finished stroke, worked out once per stroke. */
export function cachedPath(s: Stroke): string {
  let d = paths.get(s);
  if (d === undefined) {
    d = strokePath(s);
    paths.set(s, d);
  }
  return d;
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
  const b = strokeBounds(s);
  if (x < b.x0 - radius || x > b.x1 + radius || y < b.y0 - radius || y > b.y1 + radius) return false;
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

// ---- the view -----------------------------------------------------------------------------

/** A box on the plane, in ink units. */
export interface Bounds {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

const bounds = new WeakMap<Stroke, Bounds>();

/** The box a stroke's outline lies in: its points, grown by its size. */
export function strokeBounds(s: Stroke): Bounds {
  let b = bounds.get(s);
  if (b) return b;
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (let i = 0; i + 2 < s.points.length; i += 3) {
    const x = s.points[i]!;
    const y = s.points[i + 1]!;
    if (x < x0) x0 = x;
    if (x > x1) x1 = x;
    if (y < y0) y0 = y;
    if (y > y1) y1 = y;
  }
  const r = s.size;
  b = x0 === Infinity ? { x0: 0, y0: 0, x1: 0, y1: 0 } : { x0: x0 - r, y0: y0 - r, x1: x1 + r, y1: y1 + r };
  bounds.set(s, b);
  return b;
}

/** The box every stroke lies in, or null when there are none. */
export function drawingBounds(strokes: Stroke[]): Bounds | null {
  if (strokes.length === 0) return null;
  const all = strokes.map(strokeBounds);
  return {
    x0: Math.min(...all.map((b) => b.x0)),
    y0: Math.min(...all.map((b) => b.y0)),
    x1: Math.max(...all.map((b) => b.x1)),
    y1: Math.max(...all.map((b) => b.y1)),
  };
}

/** The part of the plane a view `width` by `height` screen pixels shows. */
export function viewBounds(v: InkView, width: number, height: number): Bounds {
  return { x0: v.x, y0: v.y, x1: v.x + width / v.zoom, y1: v.y + height / v.zoom };
}

/**
 * The strokes that show in the view, or come within `margin` screen pixels of it: the rest are
 * not drawn, so a large drawing pans as fast as a small one.
 */
export function visibleStrokes(strokes: Stroke[], v: InkView, width: number, height: number, margin = 64): Stroke[] {
  const m = margin / v.zoom;
  const r = viewBounds(v, width, height);
  return strokes.filter((s) => {
    const b = strokeBounds(s);
    return b.x1 >= r.x0 - m && b.x0 <= r.x1 + m && b.y1 >= r.y0 - m && b.y0 <= r.y1 + m;
  });
}

/**
 * The view a drawing opens at when it has none saved: the origin at the top-left, at 100%, or,
 * for a drawing with a page `width`, that width fitted to the column as it used to be.
 */
export function defaultView(ink: Pick<Ink, "width">, screenWidth: number): InkView {
  const zoom = ink.width && screenWidth > 0 ? Math.min(1, screenWidth / ink.width) : 1;
  return { x: 0, y: 0, zoom: clampZoom(zoom) };
}

/** The ink point under screen point (sx, sy), measured from the view's top-left. */
export function toInk(v: InkView, sx: number, sy: number): [number, number] {
  return [v.x + sx / v.zoom, v.y + sy / v.zoom];
}

/** The view moved so the plane follows a pointer that moved (dx, dy) screen pixels. */
export function panBy(v: InkView, dx: number, dy: number): InkView {
  return { x: v.x - dx / v.zoom, y: v.y - dy / v.zoom, zoom: v.zoom };
}

/** The view zoomed by `factor` about screen point (sx, sy), which stays over the same ink point. */
export function zoomAbout(v: InkView, factor: number, sx: number, sy: number): InkView {
  const zoom = clampZoom(v.zoom * factor);
  const [cx, cy] = toInk(v, sx, sy);
  return { x: cx - sx / zoom, y: cy - sy / zoom, zoom };
}

type Pt = { x: number; y: number };

/**
 * The view during a two-finger gesture that began at fingers a0, b0 on view `from` and is now at
 * a1, b1, all in screen pixels from the view's top-left: the spread zooms, and the ink point that
 * was under the fingers' midpoint stays under it as it moves.
 */
export function pinchView(from: InkView, a0: Pt, b0: Pt, a1: Pt, b1: Pt): InkView {
  const d0 = Math.hypot(a0.x - b0.x, a0.y - b0.y);
  const d1 = Math.hypot(a1.x - b1.x, a1.y - b1.y);
  const zoom = clampZoom(from.zoom * (d0 > 0 ? d1 / d0 : 1));
  const [px, py] = toInk(from, (a0.x + b0.x) / 2, (a0.y + b0.y) / 2);
  return { x: px - (a1.x + b1.x) / 2 / zoom, y: py - (a1.y + b1.y) / 2 / zoom, zoom };
}

/**
 * The view that shows every stroke in a view `width` by `height` screen pixels, with a margin,
 * no closer than 100%. A drawing without strokes fits at the origin at 100%.
 */
export function fitView(strokes: Stroke[], width: number, height: number, margin = 32): InkView {
  const b = drawingBounds(strokes);
  if (!b || width <= 0 || height <= 0) return { x: 0, y: 0, zoom: 1 };
  const bw = Math.max(1, b.x1 - b.x0);
  const bh = Math.max(1, b.y1 - b.y0);
  const zoom = clampZoom(Math.min(1, (width - 2 * margin) / bw, (height - 2 * margin) / bh));
  return { x: b.x0 - (width / zoom - bw) / 2, y: b.y0 - (height / zoom - bh) / 2, zoom };
}

/**
 * The spacing of a background's pattern, in ink units: 32, doubled as often as it takes for the
 * lines or dots to stay at least 12 screen pixels apart when zoomed out.
 */
export function patternStep(zoom: number): number {
  let step = 32;
  while (step * zoom < 12) step *= 2;
  return step;
}

// ---- input --------------------------------------------------------------------------------

export type Mode = Tool | "eraser";

/** What a press starts. */
export type Route = "draw" | "erase" | "pan" | "pinch" | "ignore";

/** A stylus's eraser end (Pointer Events: button 5, `buttons` bit 32) or its barrel button (2). */
export function stylusErases(e: { button: number; buttons: number }): boolean {
  return e.button === 5 || e.button === 2 || (e.buttons & 32) !== 0 || (e.buttons & 2) !== 0;
}

/**
 * What a pointer press on an editable drawing does (docs/vault-format.md, "Ink"):
 *
 * - A stylus always draws with the chosen tool, or erases with its eraser end or barrel button.
 * - A mouse or one finger pans, unless drawing with the mouse and touch is on, when they draw.
 *   The middle mouse button always pans.
 * - A second finger turns the gesture into a pinch, which pans and zooms; a third is ignored.
 * - While a stylus touches the surface every finger is ignored, so a resting palm does nothing.
 */
export function routePress(
  e: { pointerType: string; button: number; buttons: number },
  s: { mode: Mode; touchDraws: boolean; penDown: boolean; fingers: number },
): Route {
  const tool: Route = s.mode === "eraser" ? "erase" : "draw";
  if (e.pointerType === "pen") return stylusErases(e) ? "erase" : e.button === 0 ? tool : "ignore";
  if (e.pointerType === "touch") {
    if (s.penDown) return "ignore";
    if (s.fingers === 1) return "pinch";
    if (s.fingers > 1) return "ignore";
    return s.touchDraws ? tool : "pan";
  }
  if (e.button === 1) return "pan";
  if (e.button !== 0) return "ignore";
  return s.touchDraws ? tool : "pan";
}

// ---- colours ------------------------------------------------------------------------------

export type InkTheme = "light" | "dark";

/**
 * The colour a stroke stores for the default ink: the theme's foreground, dark on light paper and
 * light on dark paper, so a drawing reads in either theme, and on a device set to the other one.
 */
export const DEFAULT_INK = "ink";

/** The hex the default ink was stored as before the token: a pen stroke in it draws as the token. */
export const LEGACY_DEFAULT_INK = "#1f2937";

/** The paper, its guides and the default ink in each theme. */
export const PAPER: Record<InkTheme, { paper: string; guide: string; dot: string; ink: string }> = {
  light: { paper: "#ffffff", guide: "#dfe5ec", dot: "#94a3b8", ink: "#1f2937" },
  dark: { paper: "#172033", guide: "#2a364b", dot: "#56647a", ink: "#e2e8f0" },
};

/**
 * The inline palette's hues (OQ-S4): the light theme's shade, which the pen stores, and the shade
 * it draws in on dark paper, the dark theme's `--palette-*` in styles.css.
 */
const PALETTE_INK: { name: string; light: string; dark: string }[] = [
  { name: "Gray", light: "#6b7280", dark: "#9ca3af" },
  { name: "Brown", light: "#92400e", dark: "#d6a35c" },
  { name: "Orange", light: "#ea580c", dark: "#fb923c" },
  { name: "Yellow", light: "#ca8a04", dark: "#facc15" },
  { name: "Green", light: "#16a34a", dark: "#4ade80" },
  { name: "Blue", light: "#2563eb", dark: "#60a5fa" },
  { name: "Purple", light: "#9333ea", dark: "#c084fc" },
  { name: "Pink", light: "#db2777", dark: "#f472b6" },
  { name: "Red", light: "#dc2626", dark: "#f87171" },
];

const DARK_SHADE = new Map(PALETTE_INK.map((c) => [c.light, c.dark]));

type Rgb = [number, number, number];

function rgbOf(hex: string): Rgb | null {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m) return null;
  const n = parseInt(m[1]!, 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function luminance(rgb: Rgb): number {
  const lin = (c: number) => {
    const v = c / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2]);
}

/** The WCAG contrast ratio between two `#rrggbb` colours. */
export function contrast(a: string, b: string): number {
  const la = luminance(rgbOf(a) ?? [0, 0, 0]);
  const lb = luminance(rgbOf(b) ?? [0, 0, 0]);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** `hex` with its HSL lightness turned over and its hue kept: near-black becomes near-white. */
function flipLightness(hex: string): string {
  const rgb = rgbOf(hex) ?? [0, 0, 0];
  const max = Math.max(...rgb);
  const min = Math.min(...rgb);
  // At the same hue and saturation the chroma, max - min, stays, so turning L = (max + min) / 2
  // over to 1 - L moves every channel by the same amount.
  const shift = 255 - max - min;
  return `#${rgb.map((c) => (c + shift).toString(16).padStart(2, "0")).join("")}`;
}

/** Pen ink fainter than this against the paper draws with its lightness turned over. */
const MIN_PEN_CONTRAST = 2;

/**
 * The colour a stroke draws in, in `theme`. The stored colour never changes, only how it shows:
 *
 * - The default ink, and a pen stroke in the hex it was stored as before, draw in the theme's
 *   foreground.
 * - On dark paper a palette hue draws in its dark-theme shade, as inline colours do.
 * - Any other pen colour too faint against the paper, such as black on dark paper, keeps its hue
 *   with its lightness turned over.
 * - A highlighter draws in its own colour: its bright shades show on either paper.
 */
export function renderColour(colour: string, tool: Tool, theme: InkTheme): string {
  const ink = PAPER[theme].ink;
  if (colour === DEFAULT_INK || (tool === "pen" && colour === LEGACY_DEFAULT_INK)) return ink;
  const hex = colour.toLowerCase();
  if (!rgbOf(hex)) return ink;
  if (tool === "highlighter") return hex;
  if (theme === "dark" && DARK_SHADE.has(hex)) return DARK_SHADE.get(hex)!;
  return contrast(hex, PAPER[theme].paper) < MIN_PEN_CONTRAST ? flipLightness(hex) : hex;
}

// ---- tool styles --------------------------------------------------------------------------

/** A colour a tool's popover offers: its name and the value a stroke stores. */
export interface InkColour {
  name: string;
  value: string;
}

/** The ink colours offered for the pen: the default ink and the inline palette's hues (OQ-S4). */
export const PEN_COLOURS: InkColour[] = [
  { name: "Ink", value: DEFAULT_INK },
  ...PALETTE_INK.map((c) => ({ name: c.name, value: c.light })),
];

/** The highlighter's colours: the palette's hues in their bright, felt-tip shades. */
export const HIGHLIGHTER_COLOURS: InkColour[] = [
  { name: "Yellow", value: "#facc15" },
  { name: "Orange", value: "#fb923c" },
  { name: "Green", value: "#4ade80" },
  { name: "Blue", value: "#60a5fa" },
  { name: "Purple", value: "#c084fc" },
  { name: "Pink", value: "#f472b6" },
  { name: "Red", value: "#f87171" },
  { name: "Brown", value: "#d6a36a" },
  { name: "Gray", value: "#9ca3af" },
];

export interface ToolStyle {
  colour: string;
  size: number;
}

/** Each drawing tool's colours, sizes and the style it starts with. */
export const TOOL_STYLES: Record<Tool, { colours: InkColour[]; sizes: number[]; initial: ToolStyle }> = {
  pen: { colours: PEN_COLOURS, sizes: [2, 4, 8, 16], initial: { colour: DEFAULT_INK, size: 4 } },
  highlighter: { colours: HIGHLIGHTER_COLOURS, sizes: [16, 24, 36, 48], initial: { colour: "#facc15", size: 24 } },
};
