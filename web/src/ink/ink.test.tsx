import { fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Block } from "../api";
import {
  MAX_ZOOM,
  MIN_ZOOM,
  type Stroke,
  TOOL_STYLES,
  defaultView,
  fitView,
  hits,
  inkBlock,
  inkOf,
  paintOrder,
  panBy,
  patternStep,
  pinchView,
  pointerPressure,
  routePress,
  storedPoint,
  strokePath,
  toInk,
  visibleStrokes,
  zoomAbout,
} from "./ink";
import { InkSurface, VIEW_SAVE_MS, reloadTouchDraws } from "./InkSurface";

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
  beforeEach(() => {
    localStorage.clear();
    reloadTouchDraws();
  });

  function draw(svg: Element, from: [number, number], to: [number, number], pointerType = "pen", pressure = 0.5) {
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
    for (const name of ["Pen", "Highlighter", "Eraser", "Colour", "Stroke size", "Draw with mouse and touch"]) {
      const b = getByRole("button", { name });
      expect(tools.contains(b)).toBe(true);
      expect(b.getAttribute("title")).toBeTruthy();
    }
    for (const name of ["Background", "Undo", "Fit drawing", "Zoom 100%, reset to 100%"]) {
      expect(actions.contains(getByRole("button", { name }))).toBe(true);
    }

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
    // The highlighter keeps its own colour and size: yellow and wide, until changed.
    fireEvent.click(getByRole("button", { name: "Highlighter" }));
    expect(getByRole("button", { name: "Highlighter" }).getAttribute("aria-pressed")).toBe("true");
    expect(getByRole("button", { name: "Colour" }).getAttribute("title")).toBe("Colour: Yellow");
    expect(getByRole("button", { name: "Stroke size" }).getAttribute("title")).toBe("Stroke size: 24");
    draw(svg, [10, 10], [190, 10]);
    expect(block().blocks?.[0]?.fields).toMatchObject({ tool: "highlighter", colour: "#facc15", size: 24 });
    fireEvent.click(getByRole("button", { name: "Stroke size" }));
    expect(getByRole("group", { name: "Stroke sizes" }).textContent).toBe("16243648");
    fireEvent.click(getByRole("button", { name: "Size 48" }));
    fireEvent.click(getByRole("button", { name: "Colour" }));
    fireEvent.click(getByRole("button", { name: "Green" }));
    draw(svg, [10, 30], [190, 30]);
    expect(block().blocks?.[1]?.fields).toMatchObject({ tool: "highlighter", colour: "#4ade80", size: 48 });
    // Back to the pen: its own blue and 16 again.
    fireEvent.click(getByRole("button", { name: "Pen" }));
    expect(getByRole("button", { name: "Colour" }).getAttribute("title")).toBe("Colour: Blue");
    expect(getByRole("button", { name: "Stroke size" }).getAttribute("title")).toBe("Stroke size: 16");
    fireEvent.click(getByRole("button", { name: "Highlighter" }));

    // Any colour still comes from the custom picker in the popover; Escape closes it.
    fireEvent.click(getByRole("button", { name: "Colour" }));
    fireEvent.input(getByRole("group", { name: "Ink colours" }).querySelector("input[type=color]")!, {
      target: { value: "#ABCDEF" },
    });
    fireEvent.keyDown(getByRole("group", { name: "Ink colours" }), { key: "Escape" });
    expect(queryByRole("group", { name: "Ink colours" })).toBeNull();
    fireEvent.click(getByRole("button", { name: "Pen" }));
    draw(svg, [10, 50], [190, 50]);
    expect(block().blocks?.[2]?.fields).toMatchObject({ tool: "pen", colour: "#2563eb", size: 16 });
    fireEvent.click(getByRole("button", { name: "Highlighter" }));
    draw(svg, [10, 70], [190, 70]);
    expect(block().blocks?.[3]?.fields).toMatchObject({ tool: "highlighter", colour: "#abcdef", size: 48 });

    // Background: a choice from its popover, stored on the block and undoable.
    fireEvent.click(getByRole("button", { name: "Background" }));
    fireEvent.click(getByRole("button", { name: "Dots" }));
    expect(block().fields?.background).toBe("dots");
    fireEvent.click(getByRole("button", { name: "Undo" }));
    expect(block().fields?.background).toBeUndefined();
    expect(block().blocks).toHaveLength(4);
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

  it("shows a read-only drawing at its stored view, and the wheel and a drag leave it there", () => {
    const b: Block = {
      kind: "ink",
      fields: { height: 100 },
      blocks: [
        { kind: "viewport", fields: { x: -500, y: 20, zoom: 2 } },
        { kind: "stroke", fields: { tool: "pen", colour: "#1f2937", size: 4, points: [-480, 40, 0.5] } },
        { kind: "stroke", fields: { tool: "pen", colour: "#1f2937", size: 4, points: [3000, 40, 0.5] } },
      ],
    };
    const { getByRole } = render(() => <InkSurface block={b} label="Drawing" />);
    const svg = getByRole("img", { name: "Drawing" });
    expect(svg.getAttribute("viewBox")).toBe("-500 20 400 50");
    // Only the stroke in view is drawn.
    expect(svg.querySelectorAll(".ink-stroke")).toHaveLength(1);
    draw(svg, [10, 10], [50, 10], "mouse");
    fireEvent.wheel(svg, { deltaY: 100 });
    expect(svg.getAttribute("viewBox")).toBe("-500 20 400 50");
  });
});

describe("ink view", () => {
  const at = (x: number, y: number, size = 4): Stroke => ({ tool: "pen", colour: "#000000", size, points: [x, y, 0.5] });

  it("stores the view as a viewport ahead of the strokes, and leaves it out until there is one", () => {
    const block: Block = {
      kind: "ink",
      fields: { height: 300 },
      blocks: [
        { kind: "viewport", fields: { x: -1620.5, y: -400, zoom: 0.375 } },
        { kind: "stroke", fields: { tool: "pen", colour: "#1f2937", size: 4, points: [-1500.25, -320.5, 0.5] } },
      ],
    };
    const ink = inkOf(block);
    expect(ink.view).toEqual({ x: -1620.5, y: -400, zoom: 0.375 });
    expect(ink.width).toBeUndefined();
    expect(inkBlock(ink)).toEqual(block);
    expect(inkBlock({ ...ink, view: { x: 1.23456, y: -2.006, zoom: 20 } }).blocks?.[0]).toEqual({
      kind: "viewport",
      fields: { x: 1.23, y: -2.01, zoom: MAX_ZOOM },
    });
    expect(inkBlock({ ...ink, view: null }).blocks?.map((b) => b.kind)).toEqual(["stroke"]);
  });

  it("opens an old page-sized drawing with its page fitted to the column, and a new one at 100%", () => {
    expect(defaultView({ width: 800 }, 400)).toEqual({ x: 0, y: 0, zoom: 0.5 });
    expect(defaultView({ width: 800 }, 1200)).toEqual({ x: 0, y: 0, zoom: 1 });
    expect(defaultView({}, 400)).toEqual({ x: 0, y: 0, zoom: 1 });
  });

  it("pans, zooms about a point and pinches within 10% to 800%", () => {
    const v = { x: 100, y: 50, zoom: 2 };
    expect(toInk(v, 40, 20)).toEqual([120, 60]);
    expect(panBy(v, 40, -20)).toEqual({ x: 80, y: 60, zoom: 2 });
    // The ink point under the pointer stays under it.
    const z = zoomAbout(v, 2, 40, 20);
    expect(z.zoom).toBe(4);
    expect(toInk(z, 40, 20)).toEqual([120, 60]);
    expect(zoomAbout(v, 100, 0, 0).zoom).toBe(MAX_ZOOM);
    expect(zoomAbout(v, 0.0001, 0, 0).zoom).toBe(MIN_ZOOM);
    // Fingers spreading to twice as far apart, their midpoint moved 10 px right.
    const p = pinchView({ x: 0, y: 0, zoom: 1 }, { x: 90, y: 100 }, { x: 110, y: 100 }, { x: 90, y: 100 }, { x: 130, y: 100 });
    expect(p.zoom).toBe(2);
    expect(toInk(p, 110, 100)).toEqual([100, 100]);
  });

  it("fits every stroke, however far out, no closer than 100%", () => {
    const strokes = [at(-1000, -500, 0), at(1000, 500, 0)];
    const v = fitView(strokes, 432, 232, 16);
    expect(v.zoom).toBeCloseTo(0.2);
    expect(v.x).toBeCloseTo(-1080);
    expect(v.y).toBeCloseTo(-580);
    expect(fitView([at(10, 10, 0)], 400, 200).zoom).toBe(1);
    expect(fitView([], 400, 200)).toEqual({ x: 0, y: 0, zoom: 1 });
  });

  it("draws only the strokes in or near the view", () => {
    const strokes = [at(10, 10), at(5000, 10), at(-3000, -3000), at(390, 210)];
    const shown = visibleStrokes(strokes, { x: 0, y: 0, zoom: 1 }, 400, 200, 16);
    expect(shown).toEqual([strokes[0], strokes[3]]);
    expect(visibleStrokes(strokes, { x: 4800, y: -100, zoom: 1 }, 400, 200)).toEqual([strokes[1]]);
  });

  it("spaces the background pattern at least 12 screen pixels apart", () => {
    expect(patternStep(1)).toBe(32);
    expect(patternStep(0.3)).toBe(64);
    expect(patternStep(0.1)).toBe(128);
  });
});

describe("ink input", () => {
  const route = (pointerType: string, s: Partial<Parameters<typeof routePress>[1]> = {}, button = 0, buttons = 1) =>
    routePress({ pointerType, button, buttons }, { mode: "pen", touchDraws: false, penDown: false, fingers: 0, ...s });

  it("draws with a stylus and pans with the mouse and a finger by default", () => {
    expect(route("pen")).toBe("draw");
    expect(route("pen", { mode: "eraser" })).toBe("erase");
    expect(route("mouse")).toBe("pan");
    expect(route("touch")).toBe("pan");
    expect(route("mouse", {}, 1, 4)).toBe("pan");
    expect(route("mouse", {}, 2, 2)).toBe("ignore");
  });

  it("erases with a stylus's eraser end or barrel button", () => {
    expect(route("pen", {}, 5, 32)).toBe("erase");
    expect(route("pen", {}, 0, 33)).toBe("erase");
    expect(route("pen", {}, 2, 2)).toBe("erase");
  });

  it("draws with the mouse and one finger when drawing with them is on; two fingers still pinch", () => {
    expect(route("mouse", { touchDraws: true })).toBe("draw");
    expect(route("touch", { touchDraws: true })).toBe("draw");
    expect(route("touch", { touchDraws: true, mode: "eraser" })).toBe("erase");
    expect(route("mouse", { touchDraws: true }, 1, 4)).toBe("pan");
    expect(route("touch", { touchDraws: true, fingers: 1 })).toBe("pinch");
    expect(route("touch", { fingers: 1 })).toBe("pinch");
    expect(route("touch", { fingers: 2 })).toBe("ignore");
  });

  it("ignores every finger while a stylus touches the surface", () => {
    expect(route("touch", { penDown: true })).toBe("ignore");
    expect(route("touch", { penDown: true, touchDraws: true })).toBe("ignore");
    expect(route("touch", { penDown: true, fingers: 1 })).toBe("ignore");
  });

  it("starts the highlighter yellow and six times as wide as the pen, with sizes of its own", () => {
    expect(TOOL_STYLES.pen.initial).toEqual({ colour: "#1f2937", size: 4 });
    expect(TOOL_STYLES.highlighter.initial).toEqual({ colour: "#facc15", size: 24 });
    expect(TOOL_STYLES.highlighter.colours[0]).toEqual({ name: "Yellow", hex: "#facc15" });
    expect(TOOL_STYLES.highlighter.sizes).toEqual([16, 24, 36, 48]);
    expect(TOOL_STYLES.pen.sizes).toEqual([2, 4, 8, 16]);
  });

  it("draws a highlighter at one width whatever the pressure", () => {
    const hl = (pressure: number): Stroke => ({ ...line(pressure), tool: "highlighter" });
    expect(Math.abs(pathHeight(strokePath(hl(1))) - pathHeight(strokePath(hl(0.1))))).toBeLessThan(0.5);
  });
});

describe("InkSurface on the endless plane", () => {
  beforeEach(() => {
    localStorage.clear();
    reloadTouchDraws();
  });
  afterEach(() => vi.useRealTimers());

  type P = { pointerId: number; pointerType: string; x: number; y: number; buttons?: number };
  const ev = (p: P) => ({
    pointerId: p.pointerId,
    pointerType: p.pointerType,
    pressure: 0.5,
    button: 0,
    buttons: p.buttons ?? 1,
    clientX: p.x,
    clientY: p.y,
  });
  const down = (svg: Element, p: P) => fireEvent.pointerDown(svg, ev(p));
  const move = (svg: Element, p: P) => fireEvent.pointerMove(svg, ev(p));
  const up = (svg: Element, p: P) => fireEvent.pointerUp(svg, ev({ ...p, buttons: 0 }));
  function drag(svg: Element, pointerType: string, from: [number, number], to: [number, number], pointerId = 1) {
    down(svg, { pointerId, pointerType, x: from[0], y: from[1] });
    move(svg, { pointerId, pointerType, x: (from[0] + to[0]) / 2, y: (from[1] + to[1]) / 2 });
    move(svg, { pointerId, pointerType, x: to[0], y: to[1] });
    up(svg, { pointerId, pointerType, x: to[0], y: to[1] });
  }

  function surface(initial: Block = { kind: "ink", fields: { width: 200, height: 100 }, blocks: [] }) {
    const [block, setBlock] = createSignal<Block>(initial);
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
    return { ...r, block, saves, svg: r.getByRole("img", { name: "Drawing" }) };
  }

  it("pans with a mouse drag, adding no stroke, and saves the view once it settles, not as an undo step", () => {
    vi.useFakeTimers();
    const { svg, block, saves, getByRole, getByTestId } = surface();
    expect(svg.classList.contains("pans")).toBe(true);
    drag(svg, "mouse", [100, 50], [60, 20]);
    expect(svg.getAttribute("viewBox")).toBe("40 30 200 100");
    drag(svg, "mouse", [100, 50], [90, 50]);
    expect(svg.getAttribute("viewBox")).toBe("50 30 200 100");
    expect(saves).toHaveLength(0);
    vi.advanceTimersByTime(VIEW_SAVE_MS);
    expect(saves).toHaveLength(1);
    expect(block().blocks).toEqual([{ kind: "viewport", fields: { x: 50, y: 30, zoom: 1 } }]);
    expect((getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(true);
    expect(getByTestId("ink-zoom").textContent).toBe("100%");

    // A stylus stroke after panning lands where the plane is under it, and keeps the view.
    drag(svg, "pen", [0, 0], [10, 10]);
    expect(inkOf(block()).strokes[0]!.points.slice(0, 2)).toEqual([50, 30]);
    expect(block().blocks?.[0]).toEqual({ kind: "viewport", fields: { x: 50, y: 30, zoom: 1 } });
    // Undo takes the stroke back, and the view stays.
    fireEvent.click(getByRole("button", { name: "Undo" }));
    expect(block().blocks).toEqual([{ kind: "viewport", fields: { x: 50, y: 30, zoom: 1 } }]);
  });

  it("draws with the mouse and a finger once drawing with them is on, and remembers the choice", () => {
    const { svg, block, getByRole, unmount } = surface();
    const toggle = getByRole("button", { name: "Draw with mouse and touch" });
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    expect(localStorage.getItem("thevault.ink-touch-draws")).toBe("on");
    expect(svg.classList.contains("pans")).toBe(false);
    drag(svg, "mouse", [10, 10], [100, 10]);
    drag(svg, "touch", [10, 50], [100, 50], 7);
    expect(inkOf(block()).strokes.map((s) => s.points[1])).toEqual([10, 50]);
    expect(svg.getAttribute("viewBox")).toBe("0 0 200 100");
    unmount();
    reloadTouchDraws();
    const again = surface();
    expect(again.getByRole("button", { name: "Draw with mouse and touch" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("ignores a palm while the stylus draws, and drops a palm's stroke when the stylus lands", () => {
    localStorage.setItem("thevault.ink-touch-draws", "on");
    reloadTouchDraws();
    const { svg, block } = surface();
    // The palm lands first and would draw; the stylus takes over and the palm's line is dropped.
    down(svg, { pointerId: 5, pointerType: "touch", x: 150, y: 80 });
    move(svg, { pointerId: 5, pointerType: "touch", x: 160, y: 85 });
    down(svg, { pointerId: 9, pointerType: "pen", x: 10, y: 10 });
    move(svg, { pointerId: 5, pointerType: "touch", x: 170, y: 90 });
    move(svg, { pointerId: 9, pointerType: "pen", x: 50, y: 10 });
    // Another finger while the stylus is down does nothing.
    down(svg, { pointerId: 6, pointerType: "touch", x: 120, y: 60 });
    move(svg, { pointerId: 6, pointerType: "touch", x: 20, y: 60 });
    up(svg, { pointerId: 6, pointerType: "touch", x: 20, y: 60 });
    up(svg, { pointerId: 5, pointerType: "touch", x: 170, y: 90 });
    up(svg, { pointerId: 9, pointerType: "pen", x: 50, y: 10 });
    const strokes = inkOf(block()).strokes;
    expect(strokes).toHaveLength(1);
    expect(strokes[0]!.points).toEqual([10, 10, 0.5, 50, 10, 0.5]);
    expect(svg.getAttribute("viewBox")).toBe("0 0 200 100");
  });

  it("pans with one finger and pinches with two, even when fingers draw", () => {
    vi.useFakeTimers();
    localStorage.setItem("thevault.ink-touch-draws", "on");
    reloadTouchDraws();
    const { svg, block } = surface();
    down(svg, { pointerId: 1, pointerType: "touch", x: 90, y: 50 });
    move(svg, { pointerId: 1, pointerType: "touch", x: 95, y: 50 });
    down(svg, { pointerId: 2, pointerType: "touch", x: 110, y: 50 });
    // The first finger's line is dropped as the second lands; the two spread from 15 px apart to
    // 30, their middle moving from 102.5 to 95: 200%, the ink point under the middle still under it.
    move(svg, { pointerId: 1, pointerType: "touch", x: 80, y: 50 });
    move(svg, { pointerId: 2, pointerType: "touch", x: 110, y: 50 });
    expect(svg.getAttribute("data-zoom")).toBe("2");
    expect(svg.getAttribute("viewBox")).toBe("55 25 100 50");
    up(svg, { pointerId: 2, pointerType: "touch", x: 110, y: 50 });
    // The finger left pans on.
    move(svg, { pointerId: 1, pointerType: "touch", x: 60, y: 50 });
    up(svg, { pointerId: 1, pointerType: "touch", x: 60, y: 50 });
    expect(svg.getAttribute("viewBox")).toBe("65 25 100 50");
    vi.advanceTimersByTime(VIEW_SAVE_MS);
    // The first finger's line was dropped when the second landed.
    expect(inkOf(block()).strokes).toHaveLength(0);
    expect(inkOf(block()).view).toEqual({ x: 65, y: 25, zoom: 2 });
  });

  it("zooms from the keyboard and the zoom button, fits the drawing, and pans with the arrows", () => {
    const strokes: Block[] = [
      { kind: "stroke", fields: { tool: "pen", colour: "#1f2937", size: 4, points: [-900, -400, 0.5, 900, 400, 0.5] } },
    ];
    const { svg, getByRole, getByTestId } = surface({ kind: "ink", fields: { height: 100 }, blocks: strokes });
    fireEvent.keyDown(svg, { key: "+" });
    expect(getByTestId("ink-zoom").textContent).toBe("125%");
    fireEvent.click(getByTestId("ink-zoom"));
    expect(getByTestId("ink-zoom").textContent).toBe("100%");
    fireEvent.keyDown(svg, { key: "ArrowRight" });
    expect(svg.getAttribute("viewBox")).toBe("48 0 800 100");
    fireEvent.click(getByRole("button", { name: "Fit drawing" }));
    expect(getByTestId("ink-zoom").textContent).toBe("10%");
  });

  it("pans with the wheel and zooms with Ctrl+wheel about the pointer", () => {
    const { svg, getByTestId } = surface();
    fireEvent.wheel(svg, { deltaX: 30, deltaY: 20 });
    expect(svg.getAttribute("viewBox")).toBe("30 20 200 100");
    fireEvent.wheel(svg, { deltaY: -Math.log(2) / 0.0025, ctrlKey: true, clientX: 0, clientY: 0 });
    expect(getByTestId("ink-zoom").textContent).toBe("200%");
    expect(svg.getAttribute("viewBox")).toBe("30 20 100 50");
  });
});
