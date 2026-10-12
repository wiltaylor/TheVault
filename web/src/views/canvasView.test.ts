import { describe, expect, it } from "vitest";
import type { Block, Row } from "../api";
import { canvasOf } from "../canvas/canvas";
import { CANVAS_CARD, CANVAS_COLUMNS, CANVAS_GAP, layOut, placesOf, resultCanvas } from "./canvasView";

const row = (id: string): Row => ({ id, type: "task", title: id, fields: {} });

const view: Block = {
  kind: "view",
  labels: ["canvas"],
  fields: { kind: "canvas" },
  blocks: [
    { kind: "place", labels: ["b"], fields: { x: 100, y: 50, width: 300, height: 200 } },
    { kind: "place", labels: ["gone"], fields: { x: 0, y: 900 } },
    { kind: "place", labels: ["c"], fields: { x: -20, y: 10 } },
    { kind: "place", labels: ["half"], fields: { x: 5 } },
  ],
};

describe("placesOf", () => {
  it("reads each place, with the card size when it gives none, and skips one without x and y", () => {
    const places = placesOf(view);
    expect([...places.keys()]).toEqual(["b", "gone", "c"]);
    expect(places.get("b")).toEqual({ x: 100, y: 50, width: 300, height: 200 });
    expect(places.get("c")).toEqual({ x: -20, y: 10, ...CANVAS_CARD });
  });
});

describe("layOut", () => {
  it("keeps placed results where they are and lays the rest out in rows below them", () => {
    const rows = ["a", "b", "c", "d", "e", "f", "g"].map(row);
    const boxes = layOut(rows, placesOf(view));
    expect(boxes.map((b) => b.id)).toEqual(["a", "b", "c", "d", "e", "f", "g"]);
    expect(boxes[1]).toEqual({ id: "b", x: 100, y: 50, width: 300, height: 200 });
    expect(boxes[2]).toEqual({ id: "c", x: -20, y: 10, ...CANVAS_CARD });
    // The placed note "gone" is not a result, so it does not push the others down.
    const top = 50 + 200 + CANVAS_GAP;
    const step = CANVAS_CARD.width + CANVAS_GAP;
    expect(boxes[0]).toMatchObject({ x: -20, y: top });
    expect(boxes[3]).toMatchObject({ x: -20 + step, y: top });
    expect(boxes[6]).toMatchObject({ x: -20, y: top + CANVAS_CARD.height + CANVAS_GAP });
    expect(CANVAS_COLUMNS).toBe(4);
  });

  it("starts at the origin when nothing is placed", () => {
    expect(layOut([row("a")], new Map())).toEqual([{ id: "a", x: 0, y: 0, ...CANVAS_CARD }]);
  });
});

describe("resultCanvas", () => {
  it("is a canvas with one note node per result, labelled with the note id", () => {
    const c = canvasOf(resultCanvas([row("a"), row("b")], placesOf(view)));
    expect(c.viewport).toEqual({ x: 0, y: 0, zoom: 1 });
    expect(c.nodes.map((n) => [n.id, n.kind, n.block.fields?.note])).toEqual([
      ["a", "note", "a"],
      ["b", "note", "b"],
    ]);
    expect(c.nodes[1]).toMatchObject({ x: 100, y: 50, width: 300, height: 200 });
  });
});
