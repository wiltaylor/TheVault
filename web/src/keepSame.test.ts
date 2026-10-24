import { describe, expect, it } from "vitest";
import { keepSame } from "./keepSame";

const byName = (x: { name: string }) => x.name;

describe("keepSame", () => {
  it("answers the old list when nothing changed", () => {
    const prev = [{ name: "a", n: 1 }, { name: "b", n: 2 }];
    expect(keepSame(prev, [{ name: "a", n: 1 }, { name: "b", n: 2 }], byName)).toBe(prev);
  });

  it("keeps the unchanged items and takes the changed, new and reordered ones", () => {
    const a = { name: "a", n: 1 };
    const b = { name: "b", n: 2 };
    const next = [{ name: "b", n: 2 }, { name: "a", n: 9 }, { name: "c", n: 3 }];
    const out = keepSame([a, b], next, byName);
    expect(out).toEqual(next);
    expect(out[0]).toBe(b);
    expect(out[1]).toBe(next[1]);
    expect(out[2]).toBe(next[2]);
    // A reorder alone is a change too.
    const swapped = keepSame([a, b], [{ ...b }, { ...a }], byName);
    expect(swapped).not.toBe([a, b]);
    expect(swapped[0]).toBe(b);
    expect(swapped[1]).toBe(a);
  });

  it("notices a removed item", () => {
    const prev = [{ name: "a" }, { name: "b" }];
    const out = keepSame(prev, [{ name: "a" }], byName);
    expect(out).not.toBe(prev);
    expect(out[0]).toBe(prev[0]);
  });
});
