import { describe, expect, it } from "vitest";
import type { GraphDirection, GraphNode } from "../api";
import { CENTER_R, NODE_R, layoutGraph, linesOf, sortNeighbours } from "./layout";

const DIRS: GraphDirection[] = ["out", "in", "both"];

function neighbours(n: number): GraphNode[] {
  return Array.from({ length: n }, (_, i) => ({
    id: `N${String(i).padStart(3, "0")}`,
    type: i % 3 === 0 ? "task" : "page",
    title: `Note ${(i * 7) % n}`,
    updated: null,
    direction: DIRS[i % 3]!,
  }));
}

describe("graph layout", () => {
  it("puts the note in the middle and is the same every time, whatever order the neighbours come in", () => {
    const ns = neighbours(20);
    const a = layoutGraph("C", ns);
    const b = layoutGraph("C", [...ns].reverse());
    expect(a.nodes.get("C")).toEqual({ id: "C", x: 0, y: 0, r: CENTER_R, ring: 0 });
    expect([...b.nodes.entries()].sort()).toEqual([...a.nodes.entries()].sort());
    expect(a.nodes.size).toBe(21);
  });

  it("never overlaps two nodes, for any number of neighbours up to the cap", () => {
    for (let n = 0; n <= 60; n++) {
      const { nodes } = layoutGraph("C", neighbours(n));
      const all = [...nodes.values()];
      expect(all).toHaveLength(n + 1);
      for (let i = 0; i < all.length; i++) {
        for (let j = i + 1; j < all.length; j++) {
          const p = all[i]!;
          const q = all[j]!;
          const d = Math.hypot(p.x - q.x, p.y - q.y);
          expect(d, `${n} neighbours: ${p.id} and ${q.id}`).toBeGreaterThan(p.r + q.r + 8);
        }
      }
    }
  });

  it("keeps links out on the right and links in on the left", () => {
    const { nodes } = layoutGraph("C", [
      { id: "A", type: "t", title: "A", updated: null, direction: "out" },
      { id: "B", type: "t", title: "B", updated: null, direction: "in" },
    ]);
    expect(nodes.get("A")!.x).toBeGreaterThan(0);
    expect(nodes.get("B")!.x).toBeLessThan(0);
    const lone = layoutGraph("C", [{ id: "B", type: "t", title: "B", updated: null, direction: "in" }]);
    expect(lone.nodes.get("B")).toMatchObject({ x: -84, y: 0, r: NODE_R });
  });

  it("sorts neighbours out, both, in, then by type and title", () => {
    const sorted = sortNeighbours([
      { id: "1", type: "b", title: "z", updated: null, direction: "in" },
      { id: "2", type: "b", title: "a", updated: null, direction: "out" },
      { id: "3", type: "a", title: "z", updated: null, direction: "both" },
      { id: "4", type: "a", title: "y", updated: null, direction: "out" },
    ]);
    expect(sorted.map((n) => n.id)).toEqual(["4", "2", "3", "1"]);
  });

  it("merges the links between two notes into one line with both directions", () => {
    const lines = linesOf("C", [
      { from: "C", to: "A", via: "mention", field: "body" },
      { from: "C", to: "A", via: "field", field: "ref" },
      { from: "B", to: "C", via: "field", field: "about" },
      { from: "C", to: "T", via: "field", field: "tasks" },
      { from: "T", to: "C", via: "field", field: "project" },
      { from: "A", to: "B", via: "mention", field: "body" },
    ]);
    const by = (a: string, b: string) => lines.find((l) => l.a === a && l.b === b);
    expect(by("C", "A")).toMatchObject({ kind: "out", ab: true, ba: false, via: ["body (mention)", "ref"] });
    expect(by("C", "B")).toMatchObject({ kind: "in", ab: false, ba: true });
    expect(by("C", "T")).toMatchObject({ kind: "both", ab: true, ba: true });
    expect(by("A", "B")).toMatchObject({ kind: "side", ab: true, ba: false });
    expect(lines).toHaveLength(4);
  });
});
