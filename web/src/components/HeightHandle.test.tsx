import { fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import type { Block } from "../api";
import { InkSurface } from "../ink/InkSurface";
import { INK_HEIGHT, inkOf } from "../ink/ink";
import { HeightHandle, clampTo, dragHeight, keyHeight } from "./HeightHandle";

const RANGE = { initial: 480, min: 160, max: 4000 };

describe("height handle logic", () => {
  it("clamps to the bounds and rounds to whole units", () => {
    expect(clampTo(RANGE, 10)).toBe(160);
    expect(clampTo(RANGE, 99_999)).toBe(4000);
    expect(clampTo(RANGE, 600.4)).toBe(600);
    expect(clampTo(RANGE, 600.6)).toBe(601);
  });

  it("steps by 20, or 100 with Shift, and Home and End go to the bounds", () => {
    expect(keyHeight(RANGE, 480, "ArrowDown", false)).toBe(500);
    expect(keyHeight(RANGE, 480, "ArrowUp", false)).toBe(460);
    expect(keyHeight(RANGE, 480, "ArrowDown", true)).toBe(580);
    expect(keyHeight(RANGE, 480, "ArrowUp", true)).toBe(380);
    expect(keyHeight(RANGE, 170, "ArrowUp", true)).toBe(160);
    expect(keyHeight(RANGE, 3990, "ArrowDown", false)).toBe(4000);
    expect(keyHeight(RANGE, 480, "Home", false)).toBe(160);
    expect(keyHeight(RANGE, 480, "End", false)).toBe(4000);
    expect(keyHeight(RANGE, 480, "a", false)).toBeNull();
  });

  it("turns a drag in screen pixels into units at the block's scale", () => {
    expect(dragHeight(RANGE, 480, 120)).toBe(600);
    expect(dragHeight(RANGE, 400, 100, 0.5)).toBe(600);
    expect(dragHeight(RANGE, 400, -10_000, 2)).toBe(160);
    expect(dragHeight(RANGE, 400, 50, 0)).toBe(450);
  });
});

function mount(initial = 480) {
  const [height, setHeight] = createSignal(initial);
  const changes: number[] = [];
  const previews: (number | null)[] = [];
  const r = render(() => (
    <HeightHandle
      label="Block height"
      height={height()}
      range={RANGE}
      onPreview={(h) => previews.push(h)}
      onChange={(h) => {
        changes.push(h);
        setHeight(h);
      }}
    />
  ));
  return { handle: r.getByRole("separator", { name: "Block height" }), changes, previews, r };
}

const pointer = (clientY: number, extra: Record<string, unknown> = {}) => ({
  pointerId: 7,
  pointerType: "touch",
  button: 0,
  clientY,
  ...extra,
});

describe("HeightHandle", () => {
  it("previews the height while dragged and reports it once, on release", () => {
    const { handle, changes, previews, r } = mount();
    fireEvent.pointerDown(handle, pointer(100));
    fireEvent.pointerMove(handle, pointer(130));
    expect(r.getByTestId("height-readout").textContent).toBe("510 px");
    expect(handle.getAttribute("aria-valuenow")).toBe("510");
    fireEvent.pointerMove(handle, pointer(220));
    fireEvent.pointerUp(handle, pointer(220));
    expect(changes).toEqual([600]);
    expect(previews).toEqual([480, 510, 600, null]);
    expect(r.queryByTestId("height-readout")).toBeNull();
    expect(handle.getAttribute("aria-valuenow")).toBe("600");
  });

  it("reports nothing for a drag that ends where it began, or is cancelled", () => {
    const { handle, changes } = mount();
    fireEvent.pointerDown(handle, pointer(100));
    fireEvent.pointerUp(handle, pointer(100));
    fireEvent.pointerDown(handle, pointer(100));
    fireEvent.pointerMove(handle, pointer(300));
    fireEvent.pointerCancel(handle, pointer(300));
    expect(changes).toEqual([]);
  });

  it("resizes from the keyboard and resets on double-click", () => {
    const { handle, changes } = mount();
    fireEvent.keyDown(handle, { key: "ArrowDown" });
    fireEvent.keyDown(handle, { key: "ArrowDown", shiftKey: true });
    fireEvent.keyDown(handle, { key: "ArrowUp" });
    expect(changes).toEqual([500, 600, 580]);
    fireEvent.dblClick(handle);
    expect(changes).toEqual([500, 600, 580, 480]);
    // Already the default: nothing to save.
    fireEvent.dblClick(handle);
    expect(changes).toHaveLength(4);
    expect(handle.getAttribute("tabindex")).toBe("0");
  });
});

describe("ink height", () => {
  const strokes: Block[] = [10, 380].map((y) => ({
    kind: "stroke",
    fields: { tool: "pen", colour: "#1f2937", size: 4, points: [10, y, 0.5, 100, y, 0.5] },
  }));

  it("a resize changes only the height: strokes keep every coordinate, even those past the edge", () => {
    const [block, setBlock] = createSignal<Block>({ kind: "ink", fields: { width: 400, height: 400 }, blocks: strokes });
    const saves: Block[] = [];
    const r = render(() => (
      <InkSurface
        block={block()}
        label="Drawing"
        onChange={(b) => {
          saves.push(b);
          setBlock(b);
        }}
      />
    ));
    const handle = r.getByRole("separator", { name: "Drawing height" });
    const svg = r.getByRole("img", { name: "Drawing" });
    // jsdom lays nothing out, so the drawing is at 100%: one pixel per ink unit.
    fireEvent.pointerDown(handle, pointer(0));
    fireEvent.pointerMove(handle, pointer(-200));
    expect(svg.getAttribute("viewBox")).toBe("0 0 400 200");
    fireEvent.pointerUp(handle, pointer(-200));
    expect(saves).toHaveLength(1);
    expect(inkOf(block()).height).toBe(200);
    expect(block().blocks).toEqual(strokes);
    expect(svg.querySelectorAll("path.ink-stroke")).toHaveLength(2);

    fireEvent.keyDown(handle, { key: "End" });
    expect(inkOf(block()).height).toBe(INK_HEIGHT.max);
    fireEvent.dblClick(handle);
    expect(inkOf(block()).height).toBe(INK_HEIGHT.initial);
    expect(block().blocks).toEqual(strokes);
    expect(saves).toHaveLength(3);
  });

  it("a read-only drawing has no handle and shows its stored height", () => {
    const r = render(() => (
      <InkSurface block={{ kind: "ink", fields: { width: 300, height: 1200 }, blocks: [] }} label="Drawing" />
    ));
    expect(r.queryByRole("separator")).toBeNull();
    expect(r.getByRole("img", { name: "Drawing" }).getAttribute("viewBox")).toBe("0 0 300 1200");
  });
});
