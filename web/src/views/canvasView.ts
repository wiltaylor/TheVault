// A `canvas` query view (docs/vault-format.md, "Query views"; D15, D33): the results as cards on a
// canvas. A result with a `place` sits where it says; the rest are laid out in rows after the
// placed ones. The view is built as a canvas block, so the canvas renderer draws it.

import type { Block, Json, Row } from "../api";
import type { Box } from "../canvas/canvas";

/** A card's size when its `place` gives none. */
export const CANVAS_CARD = { width: 240, height: 176 };

/** The space between laid-out cards, and between them and the placed ones. */
export const CANVAS_GAP = 24;

/** Laid-out cards per row. */
export const CANVAS_COLUMNS = 4;

const num = (v: Json | undefined): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);

/** The saved positions of a canvas view, by note id. A `place` without both `x` and `y` is ignored. */
export function placesOf(view: Block | undefined): Map<string, Box> {
  const out = new Map<string, Box>();
  for (const b of view?.blocks ?? []) {
    const id = b.labels?.[0];
    const x = num(b.fields?.x);
    const y = num(b.fields?.y);
    if (b.kind !== "place" || !id || x === null || y === null) continue;
    const width = num(b.fields?.width);
    const height = num(b.fields?.height);
    out.set(id, {
      x,
      y,
      width: width !== null && width > 0 ? width : CANVAS_CARD.width,
      height: height !== null && height > 0 ? height : CANVAS_CARD.height,
    });
  }
  return out;
}

/**
 * Where each row's card goes, in row order. Placed rows keep their `place`; the others fill rows
 * of CANVAS_COLUMNS cards starting below the lowest placed card. A `place` for a note that is not
 * among `rows` is ignored.
 */
export function layOut(rows: Row[], places: Map<string, Box>): (Box & { id: string })[] {
  const placed = rows.map((r) => places.get(r.id)).filter((b): b is Box => !!b);
  const top = placed.length ? Math.max(...placed.map((b) => b.y + b.height)) + CANVAS_GAP : 0;
  const left = placed.length ? Math.min(...placed.map((b) => b.x)) : 0;
  let n = 0;
  return rows.map((r) => {
    const place = places.get(r.id);
    if (place) return { id: r.id, ...place };
    const i = n++;
    return {
      id: r.id,
      x: left + (i % CANVAS_COLUMNS) * (CANVAS_CARD.width + CANVAS_GAP),
      y: top + Math.floor(i / CANVAS_COLUMNS) * (CANVAS_CARD.height + CANVAS_GAP),
      ...CANVAS_CARD,
    };
  });
}

/** The canvas block the view draws: one `note` node per row, labelled with the note's id. */
export function resultCanvas(rows: Row[], places: Map<string, Box>): Block {
  return {
    kind: "canvas",
    blocks: [
      { kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } },
      ...layOut(rows, places).map(({ id, x, y, width, height }) => ({
        kind: "node",
        labels: [id],
        fields: { kind: "note", note: id, x, y, width, height },
      })),
    ],
  };
}
