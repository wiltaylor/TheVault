import { describe, expect, it } from "vitest";
import { isInteractive } from "./solidNodeView";

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
});
