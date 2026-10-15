import { fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { beforeEach, describe, expect, it } from "vitest";
import type { Block } from "../api";
import { canvasOf } from "./canvas";
import { CanvasSurface } from "./CanvasSurface";

const at = (x: number, y: number) => ({ pointerId: 1, pointerType: "mouse", button: 0, buttons: 1, clientX: x, clientY: y });

function drag(el: Element, from: [number, number], to: [number, number]) {
  fireEvent.pointerDown(el, at(...from));
  fireEvent.pointerMove(el, at((from[0] + to[0]) / 2, (from[1] + to[1]) / 2));
  fireEvent.pointerMove(el, at(...to));
  fireEvent.pointerUp(el, at(...to));
}

const twoNodes = (): Block => ({
  kind: "canvas",
  labels: ["board"],
  blocks: [
    { kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } },
    {
      kind: "node",
      labels: ["a"],
      fields: { kind: "card", x: 0, y: 0, width: 100, height: 50 },
      blocks: [{ kind: "paragraph", fields: { text: "**A**" } }],
    },
    {
      kind: "node",
      labels: ["b"],
      fields: { kind: "card", x: 300, y: 0, width: 100, height: 50 },
      blocks: [{ kind: "paragraph", fields: { text: "[x](https://example.com/x)" } }],
    },
  ],
});

describe("CanvasSurface", () => {
  // The zoom controls' open state lasts the browser session; each test starts with them shut.
  beforeEach(() => sessionStorage.clear());

  it("adds nodes, joins two with an edge, moves one, and keeps its label", () => {
    const [block, setBlock] = createSignal<Block>(twoNodes());
    const { getByRole, getAllByTestId, getByTestId, container } = render(() => (
      <CanvasSurface block={block()} label="Board" onChange={setBlock} />
    ));
    expect(getAllByTestId("canvas-node")).toHaveLength(2);
    expect(container.querySelector("strong")?.textContent).toBe("A");

    // No toolbar: "+" in the corner opens the popup, whose Card adds an empty card.
    expect(container.querySelector(".canvas-toolbar")).toBeNull();
    fireEvent.click(getByRole("button", { name: "Add to canvas" }));
    expect(
      [...container.querySelectorAll(".canvas-popup button")].map((b) => b.querySelector("strong")?.textContent ?? b.textContent?.trim()),
    ).toEqual(["Card", "Note", "Group"]);
    fireEvent.click(getByRole("button", { name: /Card/ }));
    expect(canvasOf(block()).nodes.map((n) => [n.id, n.kind])).toEqual([
      ["a", "card"],
      ["b", "card"],
      [expect.stringMatching(/^card_[a-z0-9]{5}$/), "card"],
    ]);

    // Dragging from a's right port and dropping on b joins them, with the closest sides saved.
    fireEvent.pointerEnter(getAllByTestId("canvas-node")[0]!);
    const port = container.querySelector('[data-testid="canvas-port"][data-node="a"][data-side="right"]')!;
    fireEvent.pointerDown(port, at(100, 25));
    fireEvent.pointerMove(getByTestId("canvas-view"), at(200, 25));
    expect(getByTestId("canvas-edge-preview")).toBeTruthy();
    fireEvent.pointerUp(getByTestId("canvas-view"), at(310, 10));
    expect(canvasOf(block()).edges).toMatchObject([{ id: expect.stringMatching(/^a_to_b_[a-z0-9]{5}$/), from: "a", to: "b", fromSide: "right", toSide: "left" }]);
    expect(getAllByTestId("canvas-edge")).toHaveLength(1);

    drag(getAllByTestId("canvas-node")[0]!, [10, 10], [50, 30]);
    expect(canvasOf(block()).nodes[0]).toMatchObject({ id: "a", x: 40, y: 20 });
    expect(block().labels).toEqual(["board"]);
  });

  it("deletes the selected node with its edges", () => {
    const start = twoNodes();
    start.blocks!.push({ kind: "edge", labels: ["e"], fields: { from: "a", to: "b" } });
    const [block, setBlock] = createSignal<Block>(start);
    const { getByRole, getAllByTestId, container } = render(() => <CanvasSurface block={block()} label="Board" onChange={setBlock} />);
    fireEvent.pointerDown(getAllByTestId("canvas-node")[0]!, at(5, 5));
    fireEvent.pointerUp(getAllByTestId("canvas-node")[0]!, at(5, 5));
    // No inspector: the selected node's own cluster holds its delete button.
    expect(container.querySelector(".canvas-inspector")).toBeNull();
    fireEvent.click(getByRole("toolbar", { name: "Card actions" }).querySelector('[aria-label="Delete"]')!);
    const c = canvasOf(block());
    expect(c.nodes.map((n) => n.id)).toEqual(["b"]);
    expect(c.edges).toEqual([]);
  });

  it("sets a selected item's colour from its palette, and clears it with No colour", () => {
    const [block, setBlock] = createSignal<Block>(twoNodes());
    const { getByRole, container } = render(() => <CanvasSurface block={block()} label="Board" onChange={setBlock} />);
    fireEvent.pointerDown(container.querySelector('[data-testid="canvas-node"][data-id="b"]')!, at(305, 5));
    fireEvent.pointerUp(container.querySelector('[data-testid="canvas-view"]')!, at(305, 5));
    fireEvent.click(getByRole("button", { name: "Colour" }));
    fireEvent.click(getByRole("button", { name: "Purple" }));
    expect(canvasOf(block()).nodes[1]?.colour).toBe("purple");
    fireEvent.click(getByRole("button", { name: "Colour" }));
    expect(getByRole("button", { name: "Purple" }).getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(getByRole("button", { name: "No colour" }));
    expect(block().blocks?.find((b) => b.labels?.[0] === "b")?.fields).not.toHaveProperty("colour");
  });

  it("sets a selected edge's arrows, and its caption", () => {
    const start = twoNodes();
    start.blocks!.push({ kind: "edge", labels: ["e"], fields: { from: "a", to: "b" } });
    const [block, setBlock] = createSignal<Block>(start);
    const { getByRole, getByLabelText, container } = render(() => <CanvasSurface block={block()} label="Board" onChange={setBlock} />);
    fireEvent.pointerDown(container.querySelector('[data-testid="canvas-edge"]')!, at(200, 25));
    fireEvent.pointerUp(container.querySelector('[data-testid="canvas-view"]')!, at(200, 25));
    const cluster = getByRole("toolbar", { name: "Connection actions" });
    for (const [name, arrow, heads] of [
      ["Arrow at start", "start", ["start"]],
      ["Arrows at both ends", "both", ["start", "end"]],
      ["No arrows", "none", []],
      ["Arrow at end", "end", ["end"]],
    ] as const) {
      fireEvent.click(cluster.querySelector(`[aria-label="${name}"]`)!);
      expect(canvasOf(block()).edges[0]?.arrow).toBe(arrow);
      expect([...container.querySelectorAll('[data-testid="canvas-arrowhead"]')].map((h) => h.getAttribute("data-end"))).toEqual(heads);
    }
    expect(block().blocks?.at(-1)?.fields).not.toHaveProperty("arrow");
    fireEvent.click(getByRole("button", { name: "Edit caption" }));
    const input = getByLabelText("Caption") as HTMLInputElement;
    fireEvent.input(input, { target: { value: "leads to" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(canvasOf(block()).edges[0]?.label).toBe("leads to");
    expect(container.querySelector('[data-testid="canvas-edge-label"]')?.textContent).toBe("leads to");
  });

  it("pans and zooms, saving the view while editing", () => {
    const [block, setBlock] = createSignal<Block>(twoNodes());
    const { getByRole, getByTestId } = render(() => <CanvasSurface block={block()} label="Board" onChange={setBlock} />);
    fireEvent.click(getByRole("button", { name: "Zoom controls" }));
    fireEvent.click(getByRole("button", { name: "Zoom in" }));
    expect(canvasOf(block()).viewport.zoom).toBe(1.25);
    expect(getByTestId("canvas-zoom").textContent).toBe("125%");
    drag(getByTestId("canvas-view"), [200, 200], [100, 150]);
    // Dragging the background 100 px left and 50 px up at 125% moves the view 80 and 40 units.
    const vp = canvasOf(block()).viewport;
    expect(vp.x - 0).toBeCloseTo(80, 1);
    expect(vp.y - 0).toBeCloseTo(40, 1);
  });

  it("moves the view without saving it, and edits nothing, when read-only", () => {
    const block = twoNodes();
    const { queryByRole, getByRole, getByTestId } = render(() => <CanvasSurface block={block} label="Board" />);
    expect(queryByRole("button", { name: "Add to canvas" })).toBeNull();
    if (!queryByRole("button", { name: "Zoom in" })) fireEvent.click(getByRole("button", { name: "Zoom controls" }));
    fireEvent.click(getByRole("button", { name: "Zoom in" }));
    expect(getByTestId("canvas-zoom").textContent).toBe("125%");
    expect(block).toEqual(twoNodes());
  });
});
