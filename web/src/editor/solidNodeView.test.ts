import { describe, expect, it } from "vitest";
import { isInteractive, refuseInteractiveDrags } from "./solidNodeView";

describe("solid node view events", () => {
  const dom = document.createElement("span");
  dom.innerHTML = `<span class="label">x</span><input /><span data-node-view-interactive><b>opt</b></span>`;
  const outside = document.createElement("input");

  it("claims events on interactive elements inside the view and leaves the rest to ProseMirror", () => {
    expect(isInteractive(dom, dom.querySelector("input"))).toBe(true);
    expect(isInteractive(dom, dom.querySelector("b"))).toBe(true);
    expect(isInteractive(dom, dom.querySelector(".label"))).toBe(false);
    expect(isInteractive(dom, dom)).toBe(false);
    expect(isInteractive(dom, outside)).toBe(false);
    expect(isInteractive(dom, null)).toBe(false);
  });

  it("refuses a drag of the node begun on an interactive element, and allows one begun elsewhere", () => {
    const node = document.createElement("div");
    node.draggable = true;
    node.innerHTML = `<p class="label">x</p><div data-node-view-interactive><canvas></canvas></div><img />`;
    refuseInteractiveDrags(node);
    const drag = (pressed: Element, target: Element) => {
      pressed.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      const e = new Event("dragstart", { bubbles: true, cancelable: true });
      target.dispatchEvent(e);
      return e.defaultPrevented;
    };
    expect(drag(node.querySelector("canvas")!, node)).toBe(true);
    expect(drag(node.querySelector(".label")!, node)).toBe(false);
    // An element inside with its own drag keeps it.
    expect(drag(node.querySelector("img")!, node.querySelector("img")!)).toBe(false);
  });
});
