import { fireEvent, render, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import type { FieldDef, Json } from "../api";

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    api: {
      ...real.api,
      note: vi.fn(async (id: string) => ({ id, type: "person", note: { fields: { title: `Person ${id}` }, blocks: [] } })),
      assets: vi.fn(async () => [{ id: "A1", filename: "plan.pdf", size: 1, path: "assets/A1.pdf" }]),
    },
  };
});

import { FieldInput, anyKind } from "./FieldInput";
import { fieldText } from "./Render";

function def(name: string, kind: string, more: Partial<FieldDef> = {}): FieldDef {
  return { name, label: name, description: null, kind, required: false, computed: false, multiple: false, compound: false, ...more };
}

function mount(field: FieldDef, start?: Json) {
  const [value, setValue] = createSignal<Json | undefined>(start);
  const r = render(() => <FieldInput field={field} value={value()} onChange={setValue} label={field.label} id={field.name} />);
  return { ...r, value, setValue };
}

const type = (el: HTMLElement, text: string) => fireEvent.input(el, { target: { value: text } });
const input = (el: HTMLElement, label: string) => within(el).getByLabelText(label, { selector: "input" }) as HTMLInputElement;

describe("list input (types T8)", () => {
  it("adds, edits, moves and removes whole numbers", () => {
    const m = mount(def("counts", "list", { label: "Counts", items: "int", item: def("counts", "int") }), [1, 2]);
    expect(m.getByLabelText("Counts item 2", { selector: "input" })).toHaveProperty("value", "2");
    fireEvent.click(m.getByRole("button", { name: "Add item" }));
    type(m.getByLabelText("Counts item 3", { selector: "input" }), "7");
    expect(m.value()).toEqual([1, 2, 7]);
    fireEvent.click(m.getByRole("button", { name: "Move Counts item 3 up" }));
    expect(m.value()).toEqual([1, 7, 2]);
    // The row moved with its input, which keeps its value.
    expect(m.getByLabelText("Counts item 2", { selector: "input" })).toHaveProperty("value", "7");
    fireEvent.click(m.getByRole("button", { name: "Remove Counts item 1" }));
    expect(m.value()).toEqual([7, 2]);
  });

  it("edits a list of date ranges, each row with its own start and end", () => {
    const m = mount(def("spans", "list", { label: "Spans", items: "date_range", item: def("spans", "date_range", { of: "date" }) }));
    fireEvent.click(m.getByRole("button", { name: "Add item" }));
    const row = m.getByRole("group", { name: "Spans item 1" });
    type(input(row, "Start"), "2026-01-01");
    expect(m.value()).toBeUndefined();
    type(input(row, "End"), "2026-01-03");
    expect(m.value()).toEqual([{ start: "2026-01-01", end: "2026-01-03" }]);
  });

  it("edits a list of dicts as records", () => {
    const m = mount(
      def("people", "list", { label: "People", items: "dict", item: def("people", "dict", { keys: [def("who", "string", { label: "Who" })], values: null }) }),
    );
    fireEvent.click(m.getByRole("button", { name: "Add item" }));
    type(within(m.getByRole("group", { name: "People item 1" })).getByLabelText("Who"), "Ann");
    expect(m.value()).toEqual([{ who: "Ann" }]);
  });
});

describe("dict input (types T8)", () => {
  const details = def("details", "dict", {
    label: "Details",
    compound: true,
    keys: [def("owner", "string", { label: "Owner", required: true }), def("size", "int", { label: "Size" })],
    values: "any",
  });

  it("edits declared keys and entries of any kind, declared keys first in their order", () => {
    const m = mount(details, { size: 3, note: "kept" });
    expect(m.getByText("(required)")).toBeTruthy();
    expect(input(m.container, "Details key 1").value).toBe("note");
    fireEvent.click(m.getByRole("button", { name: "Add entry" }));
    type(input(m.container, "Details key 2"), "flag");
    fireEvent.change(m.getByLabelText("Details kind 2"), { target: { value: "bool" } });
    expect(m.value()).toEqual({ size: 3, note: "kept", flag: false });
    fireEvent.click(m.getByLabelText("Details value 2"));
    type(m.getByLabelText("Owner"), "Bo");
    expect(m.value()).toEqual({ owner: "Bo", size: 3, note: "kept", flag: true });
    expect(Object.keys(m.value() as object)).toEqual(["owner", "size", "note", "flag"]);
    fireEvent.click(m.getByRole("button", { name: "Add entry" }));
    type(input(m.container, "Details key 3"), "tags");
    fireEvent.change(m.getByLabelText("Details kind 3"), { target: { value: "list" } });
    expect(m.getByRole("group", { name: "Details value 3" })).toBeTruthy();
    fireEvent.click(m.getByRole("button", { name: "Remove Details entry 1" }));
    expect(m.value()).toEqual({ owner: "Bo", size: 3, flag: true });
  });

  it("refuses a key name that is not an identifier or that a declared key has", () => {
    const m = mount(details);
    fireEvent.click(m.getByRole("button", { name: "Add entry" }));
    type(input(m.container, "Details key 1"), "9lives");
    type(input(m.container, "Details value 1"), "x");
    expect(m.value()).toBeUndefined();
    expect(m.getByRole("alert").textContent).toContain("letters, digits and _");
    type(input(m.container, "Details key 1"), "owner");
    expect(m.getByRole("alert").textContent).toContain("owner is a key of its own");
    type(input(m.container, "Details key 1"), "lives");
    expect(m.value()).toEqual({ lives: "x" });
  });

  it("offers no new entries when only the declared keys are allowed", () => {
    const m = mount({ ...details, values: null });
    expect(m.queryByRole("button", { name: "Add entry" })).toBeNull();
  });

  it("reads the kind of an entry of any kind from its value", () => {
    expect([true, 2, 2.5, "2026-01-01", "2026-01-01T10:00:00Z", "https://x.y", "hi", ["a"], [1], { a: 1 }].map(anyKind)).toEqual([
      "bool",
      "int",
      "float",
      "date",
      "datetime",
      "url",
      "string",
      "list",
      "other",
      "other",
    ]);
  });
});

describe("read-only lists and dicts (types T8)", () => {
  it("shows option labels, note titles and asset names inside lists and dicts", async () => {
    const f = def("details", "dict", {
      keys: [
        def("size", "select", { label: "Size", options: ["s"], option_labels: ["Small"] }),
        def("owners", "reference", { label: "Owners", multiple: true, listed: true, items: "reference", target: "person" }),
        def("files", "asset", { multiple: true, listed: true, items: "asset" }),
      ],
      values: def("details", "list", { items: "int", item: def("details", "int") }),
    });
    const value = { other: [1, 2], size: "s", owners: ["P1", "P2"], files: ["A1"] };
    // Titles and names load first; ids show until they have.
    fieldText(f, value);
    await vi.waitFor(() => expect(fieldText(f, value)).toBe("Size: Small; Owners: Person P1, Person P2; files: plan.pdf; other: 1, 2"));
    const spans = def("spans", "list", { items: "date_range", item: def("spans", "date_range") });
    expect(fieldText(spans, [{ end: "2026-01-02", start: "2026-01-01" }, { start: "2026-02-01", end: "2026-02-02" }])).toBe(
      "2026-01-01 – 2026-01-02; 2026-02-01 – 2026-02-02",
    );
  });
});
