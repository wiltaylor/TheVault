import { describe, expect, it } from "vitest";
import * as M from "./model";

/** A layout written compactly: pane ids, and `[dir, ratio, a, b]` for splits, ignoring split ids. */
type Shape = string | [M.SplitDir, number, Shape, Shape];

function shape(l: M.Layout): Shape {
  return M.isLeaf(l) ? l.pane : [l.dir, l.ratio, shape(l.a), shape(l.b)];
}

function tabShape(ws: M.Workspace, i = 0): Shape {
  return shape(ws.tabs[i]!.layout);
}

/** A workspace of one tab, its one pane at `/a`, and that pane's id. */
function one(url = "/a") {
  const ws = M.emptyWorkspace(url);
  return { ws, p: M.activePane(ws) };
}

/** One tab split into three panes: `a | (b over c)`, with ids. */
function three() {
  const { ws: w0, p: a } = one("/a");
  let ws = M.splitPane(w0, a, "right");
  const b = M.activePane(ws);
  ws = M.navigate(ws, b, "/b");
  ws = M.splitPane(ws, b, "bottom");
  const c = M.activePane(ws);
  ws = M.navigate(ws, c, "/c");
  return { ws, a, b, c };
}

describe("layout tree", () => {
  it("splits a pane right and down, the new pane showing the same page and active", () => {
    const { ws, p } = one("/notes/x");
    const right = M.splitPane(ws, p, "right");
    const q = M.activePane(right);
    expect(q).not.toBe(p);
    expect(tabShape(right)).toEqual(["row", 0.5, p, q]);
    expect(M.location(right, q)).toBe("/notes/x");

    const down = M.splitPane(right, q, "bottom");
    const r = M.activePane(down);
    expect(tabShape(down)).toEqual(["row", 0.5, p, ["column", 0.5, q, r]]);
    // The old workspace is untouched.
    expect(tabShape(right)).toEqual(["row", 0.5, p, q]);
  });

  it("splits left and top by putting the new pane first", () => {
    const { ws, p } = one();
    const left = M.splitPane(ws, p, "left");
    expect(tabShape(left)).toEqual(["row", 0.5, M.activePane(left), p]);
    const top = M.splitPane(ws, p, "top");
    expect(tabShape(top)).toEqual(["column", 0.5, M.activePane(top), p]);
  });

  it("closing a pane collapses its split and activates its sibling", () => {
    const { ws, a, b, c } = three();
    const closed = M.closePane(ws, c);
    expect(tabShape(closed)).toEqual(["row", 0.5, a, b]);
    expect(M.activePane(closed)).toBe(b);
    expect(closed.panes[c]).toBeUndefined();

    const again = M.closePane(closed, a);
    expect(tabShape(again)).toBe(b);
    expect(M.activePane(again)).toBe(b);
  });

  it("closing a tab's last pane closes the tab, and closing the last tab leaves none open", () => {
    const { ws, p } = one();
    const two = M.openTab(ws, "/b");
    expect(two.tabs).toHaveLength(2);
    const back = M.closePane(two, M.activePane(two));
    expect(back.tabs).toHaveLength(1);
    expect(M.activePane(back)).toBe(p);

    const none = M.closePane(ws, p);
    expect(M.isEmpty(none)).toBe(true);
    expect(none.tabs).toHaveLength(0);
    expect(none.panes).toEqual({});
    expect(M.activePane(none)).toBe("");
    // Nothing to cycle, and the pane actions change nothing.
    expect(M.cycleTab(none, 1)).toBe(none);
    expect(M.go(none, "", -1)).toBe(none);
    expect(M.navigate(none, "", "/x")).toBe(none);
  });

  it("an empty workspace stays empty through a reload, unless the address names a page", () => {
    const start = M.emptyWorkspace("/a");
    const none = M.closeTab(start, start.active);
    const stored = M.restore(M.serialize(M.noTabs()));
    expect(M.isEmpty(stored)).toBe(true);
    expect(M.isEmpty(M.openAtLoad(stored, "/"))).toBe(true);
    const opened = M.openAtLoad(stored, "/notes/abc");
    expect(opened.tabs).toHaveLength(1);
    expect(M.location(opened, M.activePane(opened))).toBe("/notes/abc");
    expect(M.isEmpty(none)).toBe(true);
    // Nothing stored, or nothing readable, is a first run: a daily notes tab.
    expect(M.location(M.restore(null), M.activePane(M.restore(null)))).toBe(M.HOME);
  });

  it("docks a pane on each edge of another", () => {
    const { ws, a, b, c } = three();
    expect(tabShape(M.dockPane(ws, c, a, "left"))).toEqual(["row", 0.5, ["row", 0.5, c, a], b]);
    expect(tabShape(M.dockPane(ws, c, a, "right"))).toEqual(["row", 0.5, ["row", 0.5, a, c], b]);
    expect(tabShape(M.dockPane(ws, c, a, "top"))).toEqual(["row", 0.5, ["column", 0.5, c, a], b]);
    const docked = M.dockPane(ws, c, a, "bottom");
    expect(tabShape(docked)).toEqual(["row", 0.5, ["column", 0.5, a, c], b]);
    expect(M.activePane(docked)).toBe(c);
    // Docking onto itself changes nothing.
    expect(M.dockPane(ws, c, c, "left")).toBe(ws);
  });

  it("docks a pane into another tab, closing the tab it leaves when that empties it", () => {
    const { ws: w0, p } = one("/a");
    const ws = M.openTab(w0, "/b");
    const q = M.activePane(ws);
    const docked = M.dockPane(ws, q, p, "right");
    expect(docked.tabs).toHaveLength(1);
    expect(tabShape(docked)).toEqual(["row", 0.5, p, q]);
    expect(M.location(docked, q)).toBe("/b");
  });

  it("swaps two panes, keeping their pages and histories", () => {
    const { ws, a, b, c } = three();
    const swapped = M.swapPanes(ws, a, c);
    expect(tabShape(swapped)).toEqual(["row", 0.5, c, ["column", 0.5, b, a]]);
    expect(M.location(swapped, a)).toBe("/a");
    expect(M.activePane(swapped)).toBe(a);
  });

  it("swaps panes across tabs", () => {
    const { ws: w0, p } = one("/a");
    const ws = M.openTab(w0, "/b");
    const q = M.activePane(ws);
    const swapped = M.swapPanes(ws, p, q);
    expect(tabShape(swapped, 0)).toBe(q);
    expect(tabShape(swapped, 1)).toBe(p);
    expect(swapped.active).toBe(swapped.tabs[1]!.id);
  });

  it("moves a pane into another tab, to the right of that tab's active pane", () => {
    const { ws: w0, a, b, c } = three();
    const ws = M.openTab(w0, "/d");
    const d = M.activePane(ws);
    const target = ws.tabs[1]!.id;
    const moved = M.movePaneToTab(ws, b, target);
    expect(tabShape(moved, 0)).toEqual(["row", 0.5, a, c]);
    expect(tabShape(moved, 1)).toEqual(["row", 0.5, d, b]);
    expect(moved.active).toBe(target);
    expect(M.activePane(moved)).toBe(b);
    // Into its own tab: nothing.
    expect(M.movePaneToTab(ws, d, target)).toBe(ws);
  });

  it("moves a pane to a new tab of its own", () => {
    const { ws, a, b, c } = three();
    const moved = M.movePaneToNewTab(ws, a);
    expect(moved.tabs).toHaveLength(2);
    expect(tabShape(moved, 0)).toEqual(["column", 0.5, b, c]);
    expect(tabShape(moved, 1)).toBe(a);
    expect(moved.active).toBe(moved.tabs[1]!.id);
    const first = M.movePaneToNewTab(ws, a, 0);
    expect(tabShape(first, 0)).toBe(a);
  });

  it("sets and clamps a split's ratio", () => {
    const { ws } = three();
    const root = ws.tabs[0]!.layout as Extract<M.Layout, { split: string }>;
    const wider = M.setRatio(ws, root.split, 0.7);
    expect(M.ratioOf(wider, root.split)).toBe(0.7);
    expect(M.ratioOf(M.setRatio(ws, root.split, 2), root.split)).toBe(1 - M.MIN_RATIO);
    expect(M.ratioOf(M.setRatio(ws, root.split, -1), root.split)).toBe(M.MIN_RATIO);
    expect(M.setRatio(ws, "nope", 0.3)).toBe(ws);
  });

  it("lays panes and dividers out as fractions of the tab", () => {
    const { ws, a, b, c } = three();
    const root = ws.tabs[0]!.layout as Extract<M.Layout, { split: string }>;
    const { panes, dividers } = M.layoutRects(M.setRatio(ws, root.split, 0.25).tabs[0]!.layout);
    expect(panes).toEqual([
      { pane: a, x: 0, y: 0, w: 0.25, h: 1 },
      { pane: b, x: 0.25, y: 0, w: 0.75, h: 0.5 },
      { pane: c, x: 0.25, y: 0.5, w: 0.75, h: 0.5 },
    ]);
    expect(dividers.map((d) => [d.dir, d.ratio, d.area])).toEqual([
      ["row", 0.25, { x: 0, y: 0, w: 1, h: 1 }],
      ["column", 0.5, { x: 0.25, y: 0, w: 0.75, h: 1 }],
    ]);
  });

  it("names the drop zone under a point", () => {
    expect(M.dropZone(0.05, 0.5)).toBe("left");
    expect(M.dropZone(0.95, 0.5)).toBe("right");
    expect(M.dropZone(0.5, 0.1)).toBe("top");
    expect(M.dropZone(0.5, 0.9)).toBe("bottom");
    expect(M.dropZone(0.5, 0.5)).toBe("center");
  });
});

describe("tabs", () => {
  it("opens a background tab without leaving the current one", () => {
    const { ws, p } = one();
    const bg = M.openTab(ws, "/notes/x", { background: true });
    expect(bg.tabs).toHaveLength(2);
    expect(M.activePane(bg)).toBe(p);
    expect(M.location(bg, M.activePane(M.activateTab(bg, bg.tabs[1]!.id)))).toBe("/notes/x");
  });

  it("reorders tabs and cycles through them", () => {
    let ws = M.emptyWorkspace("/a");
    ws = M.openTab(ws, "/b");
    ws = M.openTab(ws, "/c");
    const [t1, t2, t3] = ws.tabs.map((t) => t.id);
    expect(M.moveTab(ws, t3!, 0).tabs.map((t) => t.id)).toEqual([t3, t1, t2]);
    expect(M.moveTab(ws, t1!, 9).tabs.map((t) => t.id)).toEqual([t2, t3, t1]);
    expect(M.cycleTab(ws, 1).active).toBe(t1);
    expect(M.cycleTab(ws, -1).active).toBe(t2);
  });

  it("closing the active tab shows its neighbour", () => {
    let ws = M.emptyWorkspace("/a");
    ws = M.openTab(ws, "/b");
    ws = M.openTab(ws, "/c");
    const [t1, t2, t3] = ws.tabs.map((t) => t.id);
    const mid = M.closeTab(M.activateTab(ws, t2!), t2!);
    expect(mid.active).toBe(t3);
    expect(M.closeTab(ws, t3!).active).toBe(t2);
    expect(M.closeTab(ws, t1!).active).toBe(t3);
  });
});

describe("history per pane", () => {
  it("pushes, replaces, and goes back and forward within one pane only", () => {
    const { ws: w0, p } = one("/a");
    let ws = M.splitPane(w0, p, "right");
    const q = M.activePane(ws);
    ws = M.navigate(ws, p, "/b");
    ws = M.navigate(ws, p, "/c");
    expect(M.location(ws, p)).toBe("/c");
    expect(M.location(ws, q)).toBe("/a");
    ws = M.go(ws, p, -1);
    expect(M.location(ws, p)).toBe("/b");
    expect(M.canGo(ws, p, 1)).toBe(true);
    ws = M.navigate(ws, p, "/d", true);
    expect(ws.panes[p]).toEqual({ history: ["/a", "/d", "/c"], index: 1 });
    ws = M.navigate(ws, p, "/e");
    expect(ws.panes[p]).toEqual({ history: ["/a", "/d", "/e"], index: 2 });
    expect(M.go(ws, p, 5).panes[p]!.index).toBe(2);
    expect(M.go(ws, q, -1)).toBe(ws);
    expect(M.navigate(ws, p, "/e")).toBe(ws);
  });

  it("keeps at most MAX_HISTORY locations", () => {
    let { ws, p } = one("/0");
    for (let i = 1; i <= M.MAX_HISTORY + 10; i++) ws = M.navigate(ws, p, `/${i}`);
    expect(ws.panes[p]!.history).toHaveLength(M.MAX_HISTORY);
    expect(M.location(ws, p)).toBe(`/${M.MAX_HISTORY + 10}`);
  });

  it("opens the loaded address in the active pane, unless it is there already", () => {
    const { ws, p } = one("/a");
    const loaded = M.openAtLoad(ws, "/notes/x");
    expect(loaded.panes[p]).toEqual({ history: ["/a", "/notes/x"], index: 1 });
    expect(M.openAtLoad(loaded, "/notes/x")).toBe(loaded);
    expect(M.openAtLoad(ws, "/")).toBe(ws);
    expect(M.openAtLoad(ws, "/login?next=/a")).toBe(ws);
  });
});

describe("persistence", () => {
  it("round-trips a workspace of tabs, splits, ratios and histories", () => {
    const { ws: w0, b } = three();
    let ws = M.openTab(w0, "/settings");
    ws = M.navigate(ws, b, "/b2");
    ws = M.go(ws, b, -1);
    const root = ws.tabs[0]!.layout as Extract<M.Layout, { split: string }>;
    ws = M.setRatio(ws, root.split, 0.3);
    expect(M.restore(M.serialize(ws))).toEqual(ws);
    expect(M.restore(JSON.parse(M.serialize(ws)))).toEqual(ws);
  });

  it("starts fresh from nothing, garbage or another version", () => {
    for (const raw of [null, "", "{", "[]", JSON.stringify({ version: 99, tabs: [], panes: {} }), 42]) {
      const ws = M.restore(raw);
      expect(ws.tabs).toHaveLength(1);
      expect(M.location(ws, M.activePane(ws))).toBe(M.HOME);
    }
  });

  it("drops what does not hold together, collapsing splits that lose a side", () => {
    const raw = {
      version: 1,
      active: "missing",
      panes: {
        p1: { history: ["/a", "https://evil.example/", "/b"], index: 7 },
        p2: { history: ["//evil.example"], index: 0 },
        p3: { history: ["/c"], index: 0 },
        p4: { history: ["/login"], index: 0 },
        orphan: { history: ["/x"], index: 0 },
      },
      tabs: [
        { id: "t1", active: "p2", layout: { split: "s1", dir: "row", ratio: 5, a: { pane: "p1" }, b: { pane: "p2" } } },
        { id: "t2", active: "p3", layout: { split: "s2", dir: "diagonal", a: { pane: "p3" }, b: { pane: "p3" } } },
        { id: "t3", active: "p3", layout: { split: "s3", dir: "column", ratio: 0.2, a: { pane: "p3" }, b: { pane: "p3" } } },
        { id: "t1", active: "p1", layout: { pane: "p1" } },
        { id: "t4", layout: { pane: "p4" } },
        "nonsense",
      ],
    };
    const ws = M.restore(raw);
    expect(ws.tabs.map((t) => t.id)).toEqual(["t1", "t3"]);
    // p2 had no safe location: the split became p1, and p1 is active.
    expect(ws.tabs[0]).toEqual({ id: "t1", layout: { pane: "p1" }, active: "p1" });
    expect(ws.panes.p1).toEqual({ history: ["/a", "/b"], index: 1 });
    // A pane used twice keeps its first place only.
    expect(ws.tabs[1]!.layout).toEqual({ pane: "p3" });
    expect(Object.keys(ws.panes).sort()).toEqual(["p1", "p3"]);
    expect(ws.active).toBe("t1");
  });

  it("clamps a stored ratio", () => {
    const raw = {
      version: 1,
      active: "t",
      panes: { a: { history: ["/a"], index: 0 }, b: { history: ["/b"], index: 0 } },
      tabs: [{ id: "t", active: "a", layout: { split: "s", dir: "row", ratio: 1.5, a: { pane: "a" }, b: { pane: "b" } } }],
    };
    expect(M.ratioOf(M.restore(raw), "s")).toBe(1 - M.MIN_RATIO);
  });
});
