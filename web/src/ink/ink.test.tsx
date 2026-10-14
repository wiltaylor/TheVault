import { fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import type { Block } from "../api";
import { type Stroke, hits, inkBlock, inkOf, paintOrder, pointerPressure, storedPoint, strokePath } from "./ink";
import { InkSurface } from "./InkSurface";

/** The height of the box around a path's points. */
function pathHeight(d: string): number {
  const ys = [...d.matchAll(/[ML](-?[\d.]+),(-?[\d.]+)/g)].map((m) => Number(m[2]));
  return Math.max(...ys) - Math.min(...ys);
}

const line = (pressure: number): Stroke => ({
  tool: "pen",
  colour: "#000000",
  size: 8,
  points: [10, 50, pressure, 60, 50, pressure, 110, 50, pressure, 160, 50, pressure],
});

describe("ink encoding", () => {
  const block: Block = {
    kind: "ink",
    labels: ["sketch"],
    fields: { width: 400, height: 300, background: "grid" },
    blocks: [{ kind: "stroke", fields: { tool: "highlighter", colour: "#2563eb", size: 3.0, points: [1, 2, 0.5] } }],
  };

  it("round-trips a block, keeping its labels and leaving a blank background out", () => {
    expect(inkBlock(inkOf(block), block)).toEqual(block);
    const blank = inkBlock({ ...inkOf(block), background: "blank" });
    expect(blank.fields).toEqual({ width: 400, height: 300 });
    expect(blank.labels).toBeUndefined();
  });

  it("rounds coordinates to two places and pressure to three, clamped", () => {
    expect(storedPoint(1.23456, 9.87654, 0.123456)).toEqual([1.23, 9.88, 0.123]);
    expect(storedPoint(0, 0, 1.4)[2]).toBe(1);
  });

  it("stores 0.5 for a device that reports no pressure", () => {
    expect(pointerPressure({ pointerType: "mouse", pressure: 0.5 })).toBe(0.5);
    expect(pointerPressure({ pointerType: "mouse", pressure: 0 })).toBe(0.5);
    expect(pointerPressure({ pointerType: "touch", pressure: 1 })).toBe(0.5);
    expect(pointerPressure({ pointerType: "pen", pressure: 0.8 })).toBe(0.8);
    expect(pointerPressure({ pointerType: "pen", pressure: 0 })).toBe(0.5);
  });

  it("draws a heavier stroke wider", () => {
    expect(pathHeight(strokePath(line(1)))).toBeGreaterThan(pathHeight(strokePath(line(0.1))) * 1.5);
  });

  it("hits a stroke within its width plus the eraser's reach", () => {
    expect(hits(line(0.5), 85, 52, 2)).toBe(true);
    expect(hits(line(0.5), 85, 70, 2)).toBe(false);
  });

  it("paints highlighters behind pens", () => {
    const hl: Stroke = { ...line(0.5), tool: "highlighter" };
    expect(paintOrder([line(0.5), hl]).map((s) => s.index)).toEqual([1, 0]);
  });
});

describe("InkSurface", () => {
  function draw(svg: Element, from: [number, number], to: [number, number], pointerType = "mouse", pressure = 0.5) {
    const at = (x: number, y: number) => ({ pointerId: 1, pointerType, pressure, button: 0, buttons: 1, clientX: x, clientY: y });
    fireEvent.pointerDown(svg, at(...from));
    fireEvent.pointerMove(svg, at((from[0] + to[0]) / 2, (from[1] + to[1]) / 2));
    fireEvent.pointerMove(svg, at(...to));
    fireEvent.pointerUp(svg, at(...to));
  }

  it("draws strokes, erases one, and undoes the erase", () => {
    const [block, setBlock] = createSignal<Block>({ kind: "ink", fields: { width: 200, height: 100 }, blocks: [] });
    const { getByRole } = render(() => <InkSurface block={block()} label="Drawing" onChange={setBlock} />);
    const svg = getByRole("img", { name: "Drawing" });
    draw(svg, [10, 10], [190, 10]);
    draw(svg, [10, 50], [190, 50]);
    draw(svg, [10, 90], [190, 90]);
    expect(block().blocks).toHaveLength(3);
    expect(block().blocks?.[0]).toEqual({
      kind: "stroke",
      fields: { tool: "pen", colour: "#1f2937", size: 4, points: [10, 10, 0.5, 100, 10, 0.5, 190, 10, 0.5] },
    });
    expect(svg.querySelectorAll(".ink-stroke")).toHaveLength(3);

    fireEvent.click(getByRole("button", { name: "Eraser" }));
    draw(svg, [100, 48], [100, 52]);
    expect(inkOf(block()).strokes.map((s) => s.points[1])).toEqual([10, 90]);

    fireEvent.click(getByRole("button", { name: "Undo" }));
    expect(block().blocks).toHaveLength(3);
  });

  it("floats its controls on the surface: tools, colour swatches, sizes, background and undo", () => {
    const [block, setBlock] = createSignal<Block>({ kind: "ink", fields: { width: 200, height: 100 }, blocks: [] });
    const { getByRole, getByTestId, queryByRole } = render(() => (
      <InkSurface block={block()} label="Drawing" onChange={setBlock} />
    ));
    const svg = getByRole("img", { name: "Drawing" });
    const stage = svg.parentElement!;
    expect(stage.classList.contains("ink-stage")).toBe(true);
    // Both clusters sit inside the stage, over the drawing, not in a row above it.
    const tools = getByRole("toolbar", { name: "Drawing tools" });
    const actions = getByRole("toolbar", { name: "Drawing actions" });
    expect(stage.contains(tools)).toBe(true);
    expect(stage.contains(actions)).toBe(true);
    expect(getByTestId("ink-controls").parentElement).toBe(stage);
    for (const name of ["Pen", "Highlighter", "Eraser", "Colour", "Stroke size"]) {
      const b = getByRole("button", { name });
      expect(tools.contains(b)).toBe(true);
      expect(b.getAttribute("title")).toBeTruthy();
    }
    expect(actions.contains(getByRole("button", { name: "Background" }))).toBe(true);
    expect(actions.contains(getByRole("button", { name: "Undo" }))).toBe(true);

    // Colour: a swatch from the popover, which then closes.
    const colour = getByRole("button", { name: "Colour" });
    expect(colour.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(colour);
    expect(colour.getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(getByRole("button", { name: "Blue" }));
    expect(queryByRole("group", { name: "Ink colours" })).toBeNull();
    // Size: a choice from its popover.
    fireEvent.click(getByRole("button", { name: "Stroke size" }));
    fireEvent.click(getByRole("button", { name: "Size 16" }));
    fireEvent.click(getByRole("button", { name: "Highlighter" }));
    expect(getByRole("button", { name: "Highlighter" }).getAttribute("aria-pressed")).toBe("true");
    draw(svg, [10, 10], [190, 10]);
    expect(block().blocks?.[0]?.fields).toMatchObject({ tool: "highlighter", colour: "#2563eb", size: 16 });

    // Any colour still comes from the custom picker in the popover; Escape closes it.
    fireEvent.click(getByRole("button", { name: "Colour" }));
    fireEvent.input(getByRole("group", { name: "Ink colours" }).querySelector("input[type=color]")!, {
      target: { value: "#ABCDEF" },
    });
    fireEvent.keyDown(getByRole("group", { name: "Ink colours" }), { key: "Escape" });
    expect(queryByRole("group", { name: "Ink colours" })).toBeNull();
    fireEvent.click(getByRole("button", { name: "Pen" }));
    draw(svg, [10, 50], [190, 50]);
    expect(block().blocks?.[1]?.fields).toMatchObject({ tool: "pen", colour: "#abcdef", size: 16 });

    // Background: a choice from its popover, stored on the block and undoable.
    fireEvent.click(getByRole("button", { name: "Background" }));
    fireEvent.click(getByRole("button", { name: "Dots" }));
    expect(block().fields?.background).toBe("dots");
    fireEvent.click(getByRole("button", { name: "Undo" }));
    expect(block().fields?.background).toBeUndefined();
    expect(block().blocks).toHaveLength(2);
  });

  it("undoes with Ctrl+Z on the surface", () => {
    const [block, setBlock] = createSignal<Block>({ kind: "ink", fields: { width: 200, height: 100 }, blocks: [] });
    const { getByRole } = render(() => <InkSurface block={block()} label="Drawing" onChange={setBlock} />);
    const svg = getByRole("img", { name: "Drawing" });
    expect((getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(true);
    draw(svg, [10, 10], [190, 10]);
    expect((getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.keyDown(svg, { key: "z", ctrlKey: true });
    expect(block().blocks).toHaveLength(0);
  });

  it("only shows the drawing without onChange, with no controls", () => {
    const b: Block = { kind: "ink", fields: { width: 200, height: 100 }, blocks: [] };
    const { getByRole, getByTestId, queryByRole, queryByTestId } = render(() => <InkSurface block={b} label="Drawing" />);
    draw(getByRole("img", { name: "Drawing" }), [10, 10], [50, 10]);
    expect(queryByRole("toolbar")).toBeNull();
    expect(queryByRole("button")).toBeNull();
    expect(queryByTestId("ink-controls")).toBeNull();
    expect(getByTestId("ink").querySelector(".ink-cluster, .ink-action")).toBeNull();
    expect(getByRole("img").querySelectorAll(".ink-stroke")).toHaveLength(0);
  });
});
