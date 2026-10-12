import { fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
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
    { kind: "node", labels: ["a"], fields: { kind: "text", x: 0, y: 0, width: 100, height: 50, text: "**A**" } },
    { kind: "node", labels: ["b"], fields: { kind: "link", x: 300, y: 0, width: 100, height: 50, url: "https://example.com/x" } },
  ],
});

describe("CanvasSurface", () => {
  it("adds nodes, joins two with an edge, moves one, and keeps its label", () => {
    const [block, setBlock] = createSignal<Block>(twoNodes());
    const { getByRole, getAllByTestId, container } = render(() => (
      <CanvasSurface block={block()} label="Board" onChange={setBlock} />
    ));
    expect(getAllByTestId("canvas-node")).toHaveLength(2);
    expect(container.querySelector("strong")?.textContent).toBe("A");

    fireEvent.click(getByRole("button", { name: "Add text" }));
    expect(canvasOf(block()).nodes.map((n) => n.id)).toEqual(["a", "b", "text"]);

    fireEvent.click(getByRole("button", { name: "Connect" }));
    const [a, b] = getAllByTestId("canvas-node");
    fireEvent.pointerDown(a!, at(10, 10));
    fireEvent.pointerDown(b!, at(310, 10));
    expect(canvasOf(block()).edges).toMatchObject([{ id: "a_to_b", from: "a", to: "b" }]);
    expect(getAllByTestId("canvas-edge")).toHaveLength(1);
    fireEvent.click(getByRole("button", { name: "Connect" }));

    drag(getAllByTestId("canvas-node")[0]!, [10, 10], [50, 30]);
    expect(canvasOf(block()).nodes[0]).toMatchObject({ id: "a", x: 40, y: 20 });
    expect(block().labels).toEqual(["board"]);
  });

  it("deletes the selected node with its edges", () => {
    const start = twoNodes();
    start.blocks!.push({ kind: "edge", labels: ["e"], fields: { from: "a", to: "b" } });
    const [block, setBlock] = createSignal<Block>(start);
    const { getByRole, getAllByTestId } = render(() => <CanvasSurface block={block()} label="Board" onChange={setBlock} />);
    fireEvent.pointerDown(getAllByTestId("canvas-node")[0]!, at(5, 5));
    fireEvent.pointerUp(getAllByTestId("canvas-node")[0]!, at(5, 5));
    fireEvent.click(getByRole("button", { name: "Delete" }));
    const c = canvasOf(block());
    expect(c.nodes.map((n) => n.id)).toEqual(["b"]);
    expect(c.edges).toEqual([]);
  });

  it("pans and zooms, saving the view while editing", () => {
    const [block, setBlock] = createSignal<Block>(twoNodes());
    const { getByRole, getByTestId } = render(() => <CanvasSurface block={block()} label="Board" onChange={setBlock} />);
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
    expect(queryByRole("button", { name: "Add text" })).toBeNull();
    fireEvent.click(getByRole("button", { name: "Zoom in" }));
    expect(getByTestId("canvas-zoom").textContent).toBe("125%");
    expect(block).toEqual(twoNodes());
  });
});
