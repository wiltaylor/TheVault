// `columns` layout blocks (docs/vault-format.md, "columns"): two to six columns side by side, each
// with a relative width and its own blocks. Pure helpers the renderers, the editor and the
// template model share.

import type { Block } from "../api";

/** How many columns a layout holds, at least and at most. */
export const COLUMNS_MIN = 2;
export const COLUMNS_MAX = 6;

/** Below this width, in CSS pixels, a layout's columns stack (styles.css follows it). */
export const STACK_WIDTH = 560;

/** The smallest share of a pair of columns a resize leaves either of them. */
const MIN_SHARE = 0.1;

/** A width as written: positive, two decimals at most. */
export function roundWidth(w: number): number {
  return Math.max(0.01, Math.round(w * 100) / 100);
}

/** A column block's width: its `width` item when it is a positive number, else 1. */
export function columnWidth(column: Block): number {
  const w = column.fields?.width;
  return typeof w === "number" && Number.isFinite(w) && w > 0 ? w : 1;
}

/** The `column` blocks of a `columns` block. */
export function columnsOf(block: Block): Block[] {
  return (block.blocks ?? []).filter((b) => b.kind === "column");
}

/** A new layout of `n` equal, empty columns. */
export function newColumns(n: number): Block {
  const count = Math.min(COLUMNS_MAX, Math.max(COLUMNS_MIN, Math.round(n)));
  return { kind: "columns", blocks: Array.from({ length: count }, () => ({ kind: "column", fields: { width: 1 } })) };
}

/**
 * The widths of two neighbouring columns, `a` then `b`, after the gap between them moves so the
 * first takes `share` (0 to 1) of the pair's room. Their sum stays as it was, so no other column
 * changes size, and neither gets less than a tenth of the pair.
 */
export function splitPair(a: number, b: number, share: number): [number, number] {
  const sum = a + b;
  const s = Math.min(1 - MIN_SHARE, Math.max(MIN_SHARE, Number.isFinite(share) ? share : 0.5));
  const first = roundWidth(sum * s);
  return [first, roundWidth(sum - first)];
}

/** The pair's widths after the gap moves by `step`, a fraction of the pair (negative is left). */
export function nudgePair(a: number, b: number, step: number): [number, number] {
  return splitPair(a, b, a / (a + b) + step);
}

/** Equal widths for `n` columns. */
export function equalWidths(n: number): number[] {
  return Array.from({ length: n }, () => 1);
}

/** The width a column added to a layout of `widths` gets: their mean, so it takes a fair share. */
export function addedWidth(widths: number[]): number {
  return widths.length ? roundWidth(widths.reduce((s, w) => s + w, 0) / widths.length) : 1;
}

/** One template item, as a tab and the template model hold them. */
type Item = { slot: string; label: string | null } | { block: Block };

/** The field a `slot` block inside a column places. */
export function slotOfBlock(b: Block): string | null {
  return b.kind === "slot" && typeof b.labels?.[0] === "string" ? b.labels[0] : null;
}

/**
 * The fields a tab's items place with a slot, in order: slots in the tab and slots inside the
 * columns of a layout block.
 */
export function slotsOf(items: readonly Item[]): string[] {
  return items.flatMap((i) => {
    if ("slot" in i) return [i.slot];
    if (i.block.kind !== "columns") return [];
    return columnsOf(i.block).flatMap((c) => (c.blocks ?? []).flatMap((b) => slotOfBlock(b) ?? []));
  });
}
