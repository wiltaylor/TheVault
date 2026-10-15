import { describe, expect, it } from "vitest";
import type { PinItem } from "./api";
import { dropMove, findItem, folders, isWithin, keepShown, moveEntry, placeOf, removeLifting } from "./pinTree";

const note = (id: string, children: PinItem[] = []): PinItem => ({ kind: "note", id, type: "task", title: id, children });
const folder = (id: string, children: PinItem[] = []): PinItem => ({ kind: "folder", id, name: id, children });

/** The tree as `id(child child)` text. */
function shape(tree: PinItem[] | undefined): string {
  return (tree ?? [])
    .map((i) => (i.children.length ? `${i.id}(${shape(i.children)})` : i.id))
    .join(" ");
}

// a  F(b c(d))  e
const tree = (): PinItem[] => [note("a"), folder("F", [note("b"), note("c", [note("d")])]), note("e")];

describe("pinTree", () => {
  it("finds places and descendants", () => {
    expect(placeOf(tree(), "d")).toEqual({ parent: "c", index: 0 });
    expect(placeOf(tree(), "e")).toEqual({ parent: null, index: 2 });
    expect(isWithin(tree(), "F", "d")).toBe(true);
    expect(isWithin(tree(), "F", "F")).toBe(true);
    expect(isWithin(tree(), "c", "b")).toBe(false);
  });

  it("removes an entry, moving its children into its place", () => {
    expect(shape(removeLifting(tree(), "F"))).toBe("a b c(d) e");
    expect(shape(removeLifting(tree(), "c"))).toBe("a F(b d) e");
  });

  it("moves entries and refuses a move into the entry's own subtree", () => {
    expect(shape(moveEntry(tree(), "a", "F", 99))).toBe("F(b c(d) a) e");
    expect(shape(moveEntry(tree(), "e", "d", 0))).toBe("a F(b c(d(e)))");
    expect(shape(moveEntry(tree(), "F", null, 0))).toBe("F(b c(d)) a e");
    expect(moveEntry(tree(), "F", "d", 0)).toBeUndefined();
    expect(moveEntry(tree(), "c", "c", 0)).toBeUndefined();
  });

  it("turns a drop into the server's move", () => {
    const t = tree();
    expect(dropMove(t, "a", "F", "inside")).toEqual({ parent: "F", index: 2 });
    expect(dropMove(t, "a", "e", "after")).toEqual({ parent: null, index: 2 });
    expect(shape(moveEntry(t, "a", null, 2))).toBe("F(b c(d)) e a");
    expect(dropMove(t, "e", "a", "before")).toEqual({ parent: null, index: 0 });
    expect(dropMove(t, "e", "b", "after")).toEqual({ parent: "F", index: 1 });
    expect(dropMove(t, "a", null, "inside")).toEqual({ parent: null, index: 2 });
    // Dropping where it already is, or into its own subtree, does nothing.
    expect(dropMove(t, "a", "F", "before")).toBeUndefined();
    expect(dropMove(t, "e", null, "inside")).toBeUndefined();
    expect(dropMove(t, "F", "F", "inside")).toBeUndefined();
    expect(dropMove(t, "F", "d", "inside")).toBeUndefined();
    expect(dropMove(t, "F", "d", "before")).toBeUndefined();
    expect(dropMove(t, "c", "d", "after")).toBeUndefined();
  });

  it("lists folders with their depth", () => {
    const t: PinItem[] = [folder("A", [note("n", [folder("B")])]), folder("C")];
    expect(folders(t)).toEqual([
      { id: "A", name: "A", depth: 0 },
      { id: "B", name: "B", depth: 1 },
      { id: "C", name: "C", depth: 0 },
    ]);
  });

  it("keeps the shown entries a reload brings back unchanged, and takes the rest from the server", () => {
    // After a move shown at once, the server's tree is the same tree as new objects.
    const shown = moveEntry(tree(), "e", "F", 0)!;
    expect(keepShown(shown, structuredClone(shown))).toBe(shown);

    // A renamed folder is new, with its unchanged children kept; entries elsewhere are kept.
    const renamed = structuredClone(shown);
    (renamed[1] as { name: string }).name = "Renamed";
    const kept = keepShown(shown, renamed);
    expect(kept).not.toBe(shown);
    expect(kept[0]).toBe(shown[0]);
    expect(kept[1]).not.toBe(shown[1]);
    expect(kept[1]).toEqual(renamed[1]);
    expect(kept[1]!.children.every((c, i) => c === shown[1]!.children[i])).toBe(true);

    // A child moved out changes its old parent and the top level, and keeps the child itself.
    const moved = keepShown(shown, moveEntry(structuredClone(shown), "b", null, 0)!);
    expect(shape(moved)).toBe("b a F(e c(d))");
    expect(moved[0]).toBe(findItem(shown, "b"));
    expect(moved[1]).toBe(shown[0]);
    expect(moved[2]).not.toBe(shown[1]);
  });
});
