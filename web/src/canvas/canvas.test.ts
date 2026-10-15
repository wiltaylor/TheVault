import { describe, expect, it, vi } from "vitest";
import type { Block } from "../api";
import {
  addEdge,
  addGroup,
  addNode,
  canvasOf,
  closestSides,
  edgeEnds,
  elbowRoute,
  fitView,
  midpoint,
  resnapEdges,
  roundedPath,
  freeId,
  moveItem,
  newCanvas,
  removeItem,
  resizeItem,
  setItem,
  withViewport,
  zoomAbout,
  arrowEnds,
  arrowHead,
  arrowItem,
  clampHeight,
  routeDirection,
  setNodeBlocks,
  trimRoute,
  withHeight,
  CANVAS_HEIGHT,
  ARROWS,
} from "./canvas";
import { colourItem, colourLabel, isPaletteColour, paletteCss, PALETTE_COLOURS } from "./palette";

const board: Block = {
  kind: "canvas",
  labels: ["board"],
  blocks: [
    { kind: "viewport", fields: { x: -120, y: -80, zoom: 0.75 } },
    { kind: "group", labels: ["plan"], fields: { label: "Plan", x: -40, y: -40, width: 1000, height: 480, colour: "blue" } },
    {
      kind: "node",
      labels: ["goal"],
      fields: { kind: "card", x: 0, y: 0, width: 260, height: 120, group: "plan" },
      blocks: [{ kind: "paragraph", fields: { text: "Goal" } }],
    },
    {
      kind: "node",
      labels: ["sketch"],
      fields: { kind: "card", x: 1200, y: 0, width: 200, height: 100 },
      blocks: [
        {
          kind: "ink",
          fields: { width: 200, height: 100 },
          blocks: [{ kind: "stroke", fields: { tool: "pen", colour: "#000000", size: 2, points: [1, 2, 0.5] } }],
        },
      ],
    },
    { kind: "edge", labels: ["goal_to_sketch"], fields: { from: "goal", to: "sketch", arrow: "both" } },
  ],
};

describe("canvas encoding", () => {
  it("reads the viewport, groups, nodes and edges", () => {
    const c = canvasOf(board);
    expect(c.viewport).toEqual({ x: -120, y: -80, zoom: 0.75 });
    expect(c.groups).toEqual([{ id: "plan", label: "Plan", colour: "blue", x: -40, y: -40, width: 1000, height: 480 }]);
    expect(c.nodes.map((n) => [n.id, n.kind, n.group])).toEqual([
      ["goal", "card", "plan"],
      ["sketch", "card", null],
    ]);
    expect(c.edges[0]).toMatchObject({ from: "goal", to: "sketch", arrow: "both", fromSide: null });
  });

  it("reads a missing canvas as an empty one at 100%", () => {
    expect(canvasOf(undefined)).toEqual({ height: null, viewport: { x: 0, y: 0, zoom: 1 }, groups: [], nodes: [], edges: [] });
  });

  it("sets the viewport rounded, clamped and first, keeping everything else", () => {
    const next = withViewport(board, { x: 1.23456, y: -2.5, zoom: 99 });
    expect(next.blocks?.[0]).toEqual({ kind: "viewport", fields: { x: 1.23, y: -2.5, zoom: 4 } });
    expect(next.blocks?.slice(1)).toEqual(board.blocks?.slice(1));
    expect(next.labels).toEqual(["board"]);
  });
});

describe("canvas edits", () => {
  it("adds two nodes and an edge to a new canvas, with fresh ids", () => {
    const one = [{ kind: "paragraph", fields: { text: "One" } }];
    let { block, id: a } = addNode(undefined, "card", { x: 10.123, y: 20 }, {}, one);
    const second = addNode(block, "note", { x: 400, y: 20 }, { note: "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA" });
    block = second.block;
    expect(a).toMatch(/^card_[a-z0-9]{5}$/);
    expect(second.id).toMatch(/^note_[a-z0-9]{5}$/);
    const edge = addEdge(block, a, second.id);
    expect(edge.id).toMatch(new RegExp(`^${a}_to_${second.id}_[a-z0-9]{5}$`));
    expect(edge.block).toEqual({
      kind: "canvas",
      blocks: [
        { kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } },
        { kind: "node", labels: [a], fields: { kind: "card", x: 10.12, y: 20, width: 240, height: 140 }, blocks: one },
        {
          kind: "node",
          labels: [second.id],
          fields: { kind: "note", x: 400, y: 20, width: 240, height: 140, note: "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA" },
        },
        { kind: "edge", labels: [edge.id], fields: { from: a, to: second.id } },
      ],
    });
    // A card made empty holds no blocks at all.
    expect(addNode(undefined, "card", { x: 0, y: 0 }).block.blocks?.at(-1)).not.toHaveProperty("blocks");
    // The same edge again, or one from a node to itself, adds nothing.
    expect(addEdge(edge.block, a, second.id).id).toBeNull();
    expect(addEdge(edge.block, a, a).id).toBeNull();
  });

  it("puts a node made over a group's frame in that group", () => {
    expect(addNode(board, "card", { x: 100, y: 100 }).block.blocks?.at(-1)?.fields?.group).toBe("plan");
    expect(addNode(board, "card", { x: 5000, y: 100 }).block.blocks?.at(-1)?.fields?.group).toBe(undefined);
  });

  it("moves a group with its nodes, and a node into and out of a group", () => {
    const moved = canvasOf(moveItem(board, "plan", 10, 5));
    expect(moved.groups[0]).toMatchObject({ x: -30, y: -35 });
    expect(moved.nodes.find((n) => n.id === "goal")).toMatchObject({ x: 10, y: 5 });
    expect(moved.nodes.find((n) => n.id === "sketch")).toMatchObject({ x: 1200, y: 0 });

    const out = canvasOf(moveItem(board, "goal", 2000, 0)).nodes.find((n) => n.id === "goal");
    expect(out).toMatchObject({ x: 2000, group: null });
    const into = canvasOf(moveItem(board, "sketch", -1000, 50)).nodes.find((n) => n.id === "sketch");
    expect(into).toMatchObject({ x: 200, y: 50, group: "plan" });
  });

  it("keeps a card's blocks through edits", () => {
    const next = moveItem(resizeItem(board, "sketch", 10, 300), "sketch", 1, 1);
    const sketch = next.blocks?.find((b) => b.labels?.[0] === "sketch");
    expect(sketch?.blocks).toEqual(board.blocks?.[3]?.blocks);
    expect(sketch?.fields).toMatchObject({ width: 40, height: 300, x: 1201, y: 1 });
  });

  it("removes a node with its edges, and a group without its nodes", () => {
    const noGoal = canvasOf(removeItem(board, "goal"));
    expect(noGoal.nodes.map((n) => n.id)).toEqual(["sketch"]);
    expect(noGoal.edges).toEqual([]);
    const noPlan = canvasOf(removeItem(board, "plan"));
    expect(noPlan.groups).toEqual([]);
    expect(noPlan.nodes.find((n) => n.id === "goal")?.group).toBeNull();
  });

  it("sets and clears an item", () => {
    const labelled = setItem(board, "goal_to_sketch", "label", "leads to");
    expect(canvasOf(labelled).edges[0]?.label).toBe("leads to");
    expect(setItem(labelled, "goal_to_sketch", "label", undefined).blocks?.at(-1)?.fields).not.toHaveProperty("label");
    // An empty string is a value: a group's label may be empty.
    expect(setItem(board, "plan", "label", "").blocks?.[1]?.fields?.label).toBe("");
  });

  it("adds a group with a free id", () => {
    const { block, id } = addGroup(board, { x: 0, y: 600 }, "Later");
    expect(id).toMatch(/^group_[a-z0-9]{5}$/);
    expect(canvasOf(block).groups.map((g) => g.id)).toEqual(["plan", id]);
  });

  it("gives new ids a short random suffix that does not repeat across many generations", () => {
    // In one canvas, every id is free when it is made.
    let block: Block | undefined;
    const ids = new Set<string>();
    for (let i = 0; i < 2000; i++) {
      const made = addNode(block, "card", { x: 0, y: 0 });
      block = made.block;
      expect(made.id).toMatch(/^card_[a-z0-9]{5}$/);
      ids.add(made.id);
    }
    expect(ids.size).toBe(2000);
    // Across canvases, as on two devices editing offline, fresh ids still differ.
    const fresh = new Set(Array.from({ length: 200 }, () => freeId(undefined, "group")));
    expect(fresh.size).toBe(200);
    // An id that is taken is never handed out, even when the random suffix would repeat it.
    const taken: Block = { kind: "canvas", blocks: [{ kind: "group", labels: ["group_00000"] }] };
    const random = vi.spyOn(Math, "random");
    for (let i = 0; i < 5; i++) random.mockReturnValueOnce(0);
    const id = freeId(taken, "group");
    random.mockRestore();
    expect(id).toMatch(/^group_[a-z0-9]{5}$/);
    expect(id).not.toBe("group_00000");
  });

  it("leaves existing ids as they are", () => {
    const { block } = addEdge(addNode(addGroup(board, { x: 0, y: 600 }, "Later").block, "card", { x: 0, y: 0 }).block, "sketch", "goal");
    const c = canvasOf(block);
    expect(c.groups[0]?.id).toBe("plan");
    expect(c.nodes.slice(0, 2).map((n) => n.id)).toEqual(["goal", "sketch"]);
    expect(c.edges[0]?.id).toBe("goal_to_sketch");
  });
});

describe("canvas geometry", () => {
  it("zooms about a screen point, which stays over the same canvas point", () => {
    const vp = { x: 100, y: 50, zoom: 1 };
    const next = zoomAbout(vp, 2, 200, 100);
    expect(next.zoom).toBe(2);
    expect(next.x + 200 / next.zoom).toBeCloseTo(vp.x + 200 / vp.zoom);
    expect(next.y + 100 / next.zoom).toBeCloseTo(vp.y + 100 / vp.zoom);
    expect(zoomAbout(vp, 1000, 0, 0).zoom).toBe(4);
  });

  it("joins boxes at the sides facing each other unless sides are named", () => {
    const a = { x: 0, y: 0, width: 100, height: 50 };
    const b = { x: 300, y: 0, width: 100, height: 50 };
    expect(edgeEnds(a, b, null, null)).toEqual({ a: { x: 100, y: 25 }, b: { x: 300, y: 25 } });
    expect(edgeEnds(a, b, "bottom", "top")).toEqual({ a: { x: 50, y: 50 }, b: { x: 350, y: 0 } });
  });

  it("starts a new canvas at the origin", () => {
    expect(canvasOf(newCanvas()).viewport).toEqual({ x: 0, y: 0, zoom: 1 });
  });
});

describe("ports and elbow edges", () => {
  const box = (x: number, y: number, width = 100, height = 50) => ({ x, y, width, height });
  /** Every segment of a route is horizontal or vertical. */
  const orthogonal = (pts: { x: number; y: number }[]) =>
    pts.slice(1).every((p, i) => p.x === pts[i]!.x || p.y === pts[i]!.y);

  it("picks the closest pair of facing ports", () => {
    expect(closestSides(box(0, 0), box(300, 0))).toEqual({ fromSide: "right", toSide: "left" });
    expect(closestSides(box(300, 0), box(0, 0))).toEqual({ fromSide: "left", toSide: "right" });
    expect(closestSides(box(0, 0), box(0, 300))).toEqual({ fromSide: "bottom", toSide: "top" });
    expect(closestSides(box(0, 300), box(0, 0))).toEqual({ fromSide: "top", toSide: "bottom" });
    // Far right and a little lower: still side by side.
    expect(closestSides(box(0, 0), box(400, 120))).toEqual({ fromSide: "right", toSide: "left" });
    // Below and a little to the right: one over the other.
    expect(closestSides(box(0, 0), box(80, 300))).toEqual({ fromSide: "bottom", toSide: "top" });
  });

  it("routes straight between aligned facing ports", () => {
    expect(elbowRoute({ x: 100, y: 25 }, "right", { x: 300, y: 25 }, "left")).toEqual([
      { x: 100, y: 25 },
      { x: 300, y: 25 },
    ]);
  });

  it("routes an elbow that leaves and enters at right angles", () => {
    const pts = elbowRoute({ x: 100, y: 25 }, "right", { x: 400, y: 145 }, "left");
    expect(orthogonal(pts)).toBe(true);
    // Out to the right, across at the middle, in from the left.
    expect(pts).toEqual([
      { x: 100, y: 25 },
      { x: 250, y: 25 },
      { x: 250, y: 145 },
      { x: 400, y: 145 },
    ]);
    const mixed = elbowRoute({ x: 100, y: 25 }, "right", { x: 300, y: 200 }, "top");
    expect(orthogonal(mixed)).toBe(true);
    // Right out of the first box, one turn, down into the top of the second.
    expect(mixed).toEqual([
      { x: 100, y: 25 },
      { x: 300, y: 25 },
      { x: 300, y: 200 },
    ]);
  });

  it("stays orthogonal for every pair of sides", () => {
    const sides = ["top", "right", "bottom", "left"] as const;
    for (const a of sides) for (const b of sides) {
      const pts = elbowRoute({ x: 0, y: 0 }, a, { x: 137, y: -61 }, b);
      expect(orthogonal(pts), `${a} to ${b}`).toBe(true);
    }
  });

  it("rounds corners and finds the halfway point", () => {
    const d = roundedPath([{ x: 0, y: 0 }, { x: 100, y: 0 }, { x: 100, y: 100 }]);
    expect(d).toBe("M0,0 L92,0 Q100,0 100,8 L100,100");
    expect(midpoint([{ x: 0, y: 0 }, { x: 100, y: 0 }, { x: 100, y: 100 }])).toEqual({ x: 100, y: 0 });
  });

  it("writes the closest sides into every edge, and only when they change", () => {
    const b: Block = {
      kind: "canvas",
      blocks: [
        { kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } },
        { kind: "node", labels: ["a"], fields: { kind: "card", x: 0, y: 0, width: 100, height: 50 } },
        { kind: "node", labels: ["b"], fields: { kind: "card", x: 300, y: 0, width: 100, height: 50 } },
        { kind: "edge", labels: ["e"], fields: { from: "a", to: "b", from_side: "top", to_side: "top" } },
      ],
    };
    const snapped = resnapEdges(b);
    expect(canvasOf(snapped).edges[0]).toMatchObject({ fromSide: "right", toSide: "left" });
    expect(resnapEdges(snapped)).toBe(snapped);
    const below = resnapEdges(moveItem(snapped, "b", -300, 300));
    expect(canvasOf(below).edges[0]).toMatchObject({ fromSide: "bottom", toSide: "top" });
  });

  it("fits the view to everything on the canvas", () => {
    const c = canvasOf(board);
    const v = fitView(c, 800, 480);
    expect(v.zoom).toBeLessThanOrEqual(1);
    const items = [...c.nodes, ...c.groups];
    for (const it of items) {
      expect(it.x).toBeGreaterThanOrEqual(v.x);
      expect(it.x + it.width).toBeLessThanOrEqual(v.x + 800 / v.zoom + 1e-6);
    }
    expect(fitView(canvasOf(newCanvas()), 800, 480)).toEqual({ x: 0, y: 0, zoom: 1 });
  });
});

describe("cards", () => {
  it("sets a card's blocks, and an empty card holds none", () => {
    const blocks: Block[] = [
      { kind: "heading", fields: { level: 2, text: "Plan" } },
      { kind: "code", fields: { source: "ls" } },
    ];
    const next = setNodeBlocks(board, "goal", blocks);
    expect(canvasOf(next).nodes.find((n) => n.id === "goal")?.block.blocks).toEqual(blocks);
    // Nothing else changes.
    expect(next.blocks?.filter((b) => b.labels?.[0] !== "goal")).toEqual(board.blocks?.filter((b) => b.labels?.[0] !== "goal"));
    expect(setNodeBlocks(next, "goal", []).blocks?.find((b) => b.labels?.[0] === "goal")).not.toHaveProperty("blocks");
  });
});

describe("canvas height", () => {
  it("reads a stored height, clamped, and none as the default", () => {
    expect(canvasOf(board).height).toBeNull();
    expect(canvasOf({ kind: "canvas", fields: { height: 640 } }).height).toBe(640);
    expect(canvasOf({ kind: "canvas", fields: { height: 20 } }).height).toBe(CANVAS_HEIGHT.min);
    expect(canvasOf({ kind: "canvas", fields: { height: "tall" } }).height).toBeNull();
  });

  it("stores a whole height within the bounds, and the default as no item", () => {
    expect(withHeight(board, 612.6).fields).toEqual({ height: 613 });
    expect(withHeight(board, 10).fields).toEqual({ height: 160 });
    expect(withHeight(board, 99999).fields).toEqual({ height: 4000 });
    expect(withHeight(withHeight(board, 700), CANVAS_HEIGHT.initial)).not.toHaveProperty("fields");
    expect(withHeight(board, 700).blocks).toEqual(board.blocks);
    expect(withHeight(board, 700).labels).toEqual(["board"]);
    expect(clampHeight(480.4)).toBe(480);
  });
});

describe("arrows", () => {
  it("maps each arrow choice to its ends and its stored item", () => {
    expect(ARROWS).toEqual(["none", "start", "end", "both"]);
    expect(ARROWS.map(arrowEnds)).toEqual([
      { start: false, end: false },
      { start: true, end: false },
      { start: false, end: true },
      { start: true, end: true },
    ]);
    // `end` is the default, so it is stored as no item.
    expect(ARROWS.map(arrowItem)).toEqual(["none", "start", undefined, "both"]);
    const at = (arrow: string) =>
      canvasOf({ kind: "canvas", blocks: [{ kind: "edge", labels: ["e"], fields: { from: "a", to: "b", arrow } }] }).edges[0]?.arrow;
    expect(["none", "start", "end", "both", "sideways"].map(at)).toEqual(["none", "start", "end", "both", "end"]);
  });

  it("draws an arrowhead with its tip on the end of the line, pointing along it", () => {
    expect(arrowHead({ x: 0, y: 0 }, { x: 100, y: 0 })).toBe("100,0 90,4.5 90,-4.5");
    expect(arrowHead({ x: 0, y: 100 }, { x: 0, y: 0 })).toBe("0,0 4.5,10 -4.5,10");
    // The line stops short of an arrowed tip, and only there.
    const pts = [
      { x: 0, y: 0 },
      { x: 50, y: 0 },
      { x: 50, y: 50 },
    ];
    expect(trimRoute(pts, true, false)).toEqual([{ x: 4, y: 0 }, pts[1], pts[2]]);
    expect(trimRoute(pts, false, true)).toEqual([pts[0], pts[1], { x: 50, y: 46 }]);
    expect(trimRoute(pts, false, false)).toEqual(pts);
  });

  it("gives the direction a route runs, from its start to its end, along the main axis between them", () => {
    const right = [
      { x: 0, y: 0 },
      { x: 100, y: 0 },
    ];
    expect(routeDirection(right)).toBe(0);
    expect(routeDirection([...right].reverse())).toBe(180);
    expect(
      routeDirection([
        { x: 0, y: 0 },
        { x: 10, y: 0 },
        { x: 10, y: 200 },
        { x: 20, y: 200 },
      ]),
    ).toBe(90);
    expect(
      routeDirection([
        { x: 0, y: 200 },
        { x: 0, y: 0 },
      ]),
    ).toBe(270);
    // A jog longer than the runs either side of it does not turn a mostly vertical route.
    expect(
      routeDirection([
        { x: 140, y: 170 },
        { x: 140, y: 200 },
        { x: 180, y: 200 },
        { x: 180, y: 230 },
      ]),
    ).toBe(90);
    // A small jog in the middle of a long level run does not turn it.
    expect(
      routeDirection([
        { x: 0, y: 0 },
        { x: 100, y: 0 },
        { x: 100, y: 4 },
        { x: 220, y: 4 },
      ]),
    ).toBe(0);
  });
});

describe("palette", () => {
  it("knows the palette names and nothing else", () => {
    expect(PALETTE_COLOURS).toEqual(["gray", "brown", "orange", "yellow", "green", "blue", "purple", "pink", "red"]);
    expect(isPaletteColour("red")).toBe(true);
    expect(isPaletteColour("crimson")).toBe(false);
    expect(isPaletteColour(null)).toBe(false);
  });

  it("draws a colour from the theme, and stores no item for no colour", () => {
    expect(paletteCss("blue")).toBe("var(--palette-blue)");
    expect(paletteCss(null)).toBeUndefined();
    expect(paletteCss("#ff0000")).toBeUndefined();
    expect(colourItem("green")).toBe("green");
    expect(colourItem(null)).toBeUndefined();
    expect(colourItem("chartreuse")).toBeUndefined();
    expect(colourLabel("gray")).toBe("Gray");
    expect(colourLabel(null)).toBe("No colour");
  });
});
