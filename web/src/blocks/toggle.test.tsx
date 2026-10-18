import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Block } from "../api";
import { Blocks } from "../components/Render";
import { ADMONITION_KINDS, PRESETS, admonitionLook, holdsFind, nestedText, readOpen, setFindText, writeOpen } from "./toggle";

const p = (text: string): Block => ({ kind: "paragraph", fields: { text } });
const toggle = (text: string, ...blocks: Block[]): Block => ({ kind: "toggle", fields: { text }, blocks });

beforeEach(() => localStorage.clear());
afterEach(() => setFindText("", null));

describe("toggle open state", () => {
  it("is remembered per scope and path on this device, and nothing is kept without a scope", () => {
    expect(readOpen("N/body", "0", false)).toBe(false);
    writeOpen("N/body", "0", true);
    expect(readOpen("N/body", "0", false)).toBe(true);
    expect(readOpen("N/body", "1", false)).toBe(false);
    expect(readOpen("M/body", "0", false)).toBe(false);
    writeOpen(undefined, "0", true);
    expect(readOpen(undefined, "0", false)).toBe(false);
  });

  it("matches a search by every word of it, in the nested blocks' text", () => {
    const text = nestedText([p("Alpha **beta**"), { kind: "list", fields: { style: "bullet" }, blocks: [{ kind: "item", fields: { text: "Gamma" } }] }]);
    expect(holdsFind(text, "beta gamma")).toBe(true);
    expect(holdsFind(text, "beta delta")).toBe(false);
    expect(holdsFind(text, null)).toBe(false);
  });
});

describe("ToggleView", () => {
  const blocks = [toggle("Summary **bold**", p("hidden inside"))];

  it("starts closed, opens and closes", () => {
    const { container, getByTestId, queryByText } = render(() => <Blocks blocks={blocks} scope="N/body" />);
    expect(container.querySelector(".toggle-text strong")?.textContent).toBe("bold");
    expect(queryByText("hidden inside")).toBeNull();
    const button = getByTestId("toggle-button");
    expect(button.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(button);
    expect(queryByText("hidden inside")).not.toBeNull();
    expect(button.getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(button);
    expect(queryByText("hidden inside")).toBeNull();
  });

  it("remembers its state across a reload on the same device", () => {
    const first = render(() => <Blocks blocks={blocks} scope="N/body" />);
    fireEvent.click(first.getByTestId("toggle-button"));
    first.unmount();
    const again = render(() => <Blocks blocks={blocks} scope="N/body" />);
    expect(again.queryByText("hidden inside")).not.toBeNull();
    again.unmount();
    // Another note's toggle at the same place is its own.
    const other = render(() => <Blocks blocks={blocks} scope="M/body" />);
    expect(other.queryByText("hidden inside")).toBeNull();
  });

  it("shows a heading summary at its level", () => {
    const { container } = render(() => <Blocks blocks={[{ kind: "toggle", fields: { level: 2, text: "Big" } }]} />);
    expect(container.querySelector("h3.toggle-text")?.textContent).toBe("Big");
  });

  it("opens for a search hit inside it, and only in the note searched", async () => {
    setFindText("N", "inside");
    const { queryByText, unmount } = render(() => <Blocks blocks={[toggle("a", p("x")), ...blocks]} scope="N/body" />);
    await waitFor(() => expect(queryByText("hidden inside")).not.toBeNull());
    expect(queryByText("x")).toBeNull();
    unmount();
    const other = render(() => <Blocks blocks={blocks} scope="M/body" />);
    expect(other.queryByText("hidden inside")).toBeNull();
  });

  it("opens a toggle nested in a column or another toggle by its own place", () => {
    const nested: Block[] = [p("top"), toggle("outer", toggle("inner", p("deep")))];
    writeOpen("N/body", "1", true);
    writeOpen("N/body", "1.0", true);
    const { queryByText } = render(() => <Blocks blocks={nested} scope="N/body" />);
    expect(queryByText("deep")).not.toBeNull();
  });
});

describe("AdmonitionView", () => {
  it("renders each preset's colour, icon and label", () => {
    for (const kind of ADMONITION_KINDS) {
      const { getByTestId, unmount } = render(() => <Blocks blocks={[{ kind: "admonition", fields: { kind }, blocks: [p("body")] }]} />);
      const box = getByTestId("admonition");
      expect(box.dataset.colour).toBe(PRESETS[kind].colour);
      expect(box.dataset.icon).toBe(PRESETS[kind].icon);
      expect(box.style.getPropertyValue("--admonition")).toBe(`var(--palette-${PRESETS[kind].colour})`);
      expect(box.querySelector(".admonition-title")?.textContent).toBe(PRESETS[kind].label);
      expect(box.querySelector(".admonition-icon svg")).not.toBeNull();
      unmount();
    }
    expect(PRESETS.warning).toEqual({ colour: "orange", icon: "triangle-alert", label: "Warning" });
    expect(PRESETS.danger).toEqual({ colour: "red", icon: "octagon-alert", label: "Danger" });
  });

  it("takes an override of colour, icon and title", () => {
    const block: Block = { kind: "admonition", fields: { kind: "warning", colour: "pink", icon: "flame", title: "Hot **stuff**" } };
    const { getByTestId } = render(() => <Blocks blocks={[block]} />);
    const box = getByTestId("admonition");
    expect(box.dataset.kind).toBe("warning");
    expect(box.dataset.colour).toBe("pink");
    expect(box.dataset.icon).toBe("flame");
    expect(box.style.getPropertyValue("--admonition")).toBe("var(--palette-pink)");
    expect(box.querySelector(".admonition-title strong")?.textContent).toBe("stuff");
    expect(admonitionLook({ kind: "nonsense" }).kind).toBe("note");
  });

  it("opens and closes when collapsible, starting open; otherwise has no button", () => {
    const plain = render(() => <Blocks blocks={[{ kind: "admonition", fields: { kind: "tip" }, blocks: [p("always")] }]} />);
    expect(plain.queryByTestId("toggle-button")).toBeNull();
    expect(plain.queryByText("always")).not.toBeNull();
    plain.unmount();
    const folding = render(() => (
      <Blocks blocks={[{ kind: "admonition", fields: { kind: "tip", collapsible: true }, blocks: [p("folds")] }]} scope="N/body" />
    ));
    expect(folding.queryByText("folds")).not.toBeNull();
    fireEvent.click(folding.getByTestId("toggle-button"));
    expect(folding.queryByText("folds")).toBeNull();
    expect(folding.getByTestId("admonition").dataset.open).toBe("false");
    // The title stays.
    expect(folding.getByText("Tip")).not.toBeNull();
    fireEvent.click(folding.getByTestId("toggle-button"));
    expect(folding.queryByText("folds")).not.toBeNull();
  });
});
