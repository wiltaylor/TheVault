import { describe, expect, it } from "vitest";
import type { Block } from "../api";
import {
  addEdge,
  addGroup,
  addNode,
  canvasOf,
  edgeEnds,
  freeId,
  moveItem,
  newCanvas,
  removeItem,
  resizeItem,
  setItem,
  withViewport,
  zoomAbout,
} from "./canvas";

const board: Block = {
  kind: "canvas",
  labels: ["board"],
  blocks: [
    { kind: "viewport", fields: { x: -120, y: -80, zoom: 0.75 } },
    { kind: "group", labels: ["plan"], fields: { label: "Plan", x: -40, y: -40, width: 1000, height: 480, colour: "blue" } },
    { kind: "node", labels: ["goal"], fields: { kind: "text", x: 0, y: 0, width: 260, height: 120, group: "plan", text: "Goal" } },
    {
      kind: "node",
      labels: ["sketch"],
      fields: { kind: "ink", x: 1200, y: 0, width: 200, height: 100 },
      blocks: [{ kind: "stroke", fields: { tool: "pen", colour: "#000000", size: 2, points: [1, 2, 0.5] } }],
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
      ["goal", "text", "plan"],
      ["sketch", "ink", null],
    ]);
    expect(c.edges[0]).toMatchObject({ from: "goal", to: "sketch", arrow: "both", fromSide: null });
  });

  it("reads a missing canvas as an empty one at 100%", () => {
    expect(canvasOf(undefined)).toEqual({ viewport: { x: 0, y: 0, zoom: 1 }, groups: [], nodes: [], edges: [] });
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
    let { block, id: a } = addNode(undefined, "text", { x: 10.123, y: 20 }, { text: "One" });
    const second = addNode(block, "text", { x: 400, y: 20 }, { text: "Two" });
    block = second.block;
    expect([a, second.id]).toEqual(["text", "text_2"]);
    const edge = addEdge(block, a, second.id);
    expect(edge.id).toBe("text_to_text_2");
    expect(edge.block).toEqual({
      kind: "canvas",
      blocks: [
        { kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } },
        { kind: "node", labels: ["text"], fields: { kind: "text", x: 10.12, y: 20, width: 240, height: 140, text: "One" } },
        { kind: "node", labels: ["text_2"], fields: { kind: "text", x: 400, y: 20, width: 240, height: 140, text: "Two" } },
        { kind: "edge", labels: ["text_to_text_2"], fields: { from: "text", to: "text_2" } },
      ],
    });
    // The same edge again, or one from a node to itself, adds nothing.
    expect(addEdge(edge.block, a, second.id).id).toBeNull();
    expect(addEdge(edge.block, a, a).id).toBeNull();
  });

  it("puts a node made over a group's frame in that group", () => {
    expect(addNode(board, "link", { x: 100, y: 100 }, { url: "https://example.com" }).block.blocks?.at(-1)?.fields?.group).toBe(
      "plan",
    );
    expect(addNode(board, "link", { x: 5000, y: 100 }, { url: "https://example.com" }).block.blocks?.at(-1)?.fields?.group).toBe(
      undefined,
    );
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

  it("keeps an ink node's strokes through edits", () => {
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
    // An empty string is a value: a text node's required text may be empty.
    expect(setItem(board, "goal", "text", "").blocks?.[2]?.fields?.text).toBe("");
  });

  it("adds a group with a free id", () => {
    const { block, id } = addGroup(board, { x: 0, y: 600 }, "Later");
    expect(id).toBe("group");
    expect(freeId(block, "group")).toBe("group_2");
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
