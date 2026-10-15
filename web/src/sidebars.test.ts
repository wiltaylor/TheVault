import { describe, expect, it } from "vitest";
import {
  LEFT_DEFAULT,
  MAX_WIDTH,
  MIN_WIDTH,
  RIGHT_DEFAULT,
  clampWidth,
  defaultSidebars,
  readSidebars,
  setWidth,
  showPanel,
  toggleSide,
} from "./sidebars";

describe("sidebars", () => {
  it("start with the left open and the right collapsed on Properties", () => {
    expect(defaultSidebars()).toEqual({
      left: { open: true, width: LEFT_DEFAULT },
      right: { open: false, width: RIGHT_DEFAULT, panel: "properties", graph: true },
    });
  });

  it("toggle, open on a panel, and clamp widths", () => {
    const d = defaultSidebars();
    expect(toggleSide(d, "left").left.open).toBe(false);
    expect(toggleSide(d, "right").right.open).toBe(true);
    expect(toggleSide(d, "right", false)).toEqual(d);
    expect(showPanel(d, "properties").right).toEqual({ open: true, width: RIGHT_DEFAULT, panel: "properties", graph: true });
    expect(showPanel(d, "history").right).toEqual({ open: true, width: RIGHT_DEFAULT, panel: "history", graph: true });
    expect(readSidebars(JSON.stringify(showPanel(d, "history"))).right.panel).toBe("history");
    expect(setWidth(d, "left", 10).left.width).toBe(MIN_WIDTH);
    expect(setWidth(d, "right", 9999).right.width).toBe(MAX_WIDTH);
    expect(clampWidth(300.4)).toBe(300);
  });

  it("round-trip through storage, and fall back to defaults for anything malformed", () => {
    const s = setWidth(toggleSide(toggleSide(defaultSidebars(), "left"), "right"), "right", 400);
    expect(readSidebars(JSON.stringify(s))).toEqual(s);
    const folded = { ...s, right: { ...s.right, graph: false } };
    expect(readSidebars(JSON.stringify(folded)).right.graph).toBe(false);
    expect(readSidebars(null)).toEqual(defaultSidebars());
    expect(readSidebars("{oops")).toEqual(defaultSidebars());
    expect(readSidebars({ left: { open: "yes", width: "wide" }, right: { open: true, width: 5, panel: "chat", graph: "no" } })).toEqual({
      left: { open: true, width: LEFT_DEFAULT },
      right: { open: true, width: MIN_WIDTH, panel: "properties", graph: true },
    });
  });
});
