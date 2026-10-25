import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FieldDef, Json } from "../api";

vi.mock("../clipboard", () => ({ copyText: vi.fn(async () => true) }));
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    api: {
      ...real.api,
      query: vi.fn(async () => ({
        rows: [{ id: "x", type: "person", title: "X", fields: { phones: [{ number: "+61400000099", label: "pager" }, { number: "+61400000098" }] } }],
      })),
    },
  };
});

import { copyText } from "../clipboard";
import { FieldInput } from "./FieldInput";
import { addTo, editAt, labelled, labelledShape, labelledText, labelledValue, removeAt } from "./LabelledList";
import { fieldText } from "./Render";

function def(name: string, kind: string, more: Partial<FieldDef> = {}): FieldDef {
  return { name, label: name, description: null, kind, required: false, computed: false, multiple: false, compound: false, ...more };
}

/** A list whose items are a dict of `keys`, with no other entries unless `values` says so. */
function listOf(keys: FieldDef[], values: FieldDef["values"] = null): FieldDef {
  return def("phones", "list", { label: "Phones", items: "dict", item: def("phones", "dict", { keys, values }) });
}

const PHONES = listOf([def("number", "string", { required: true }), def("label", "string")]);

/** A person note's phones in a signal, edited through FieldInput as the note page does. */
function mount(start: Json | undefined) {
  const [value, setValue] = createSignal<Json | undefined>(start);
  const writes: (Json | undefined)[] = [];
  const r = render(() => (
    <FieldInput
      field={PHONES}
      value={value()}
      related={{ type: "person" }}
      onChange={(v) => {
        writes.push(v);
        setValue(v);
      }}
      label="Phones"
      id="phones"
    />
  ));
  const chips = () =>
    [...r.container.querySelectorAll(".labelled-chip")].map((c) => [
      c.querySelector(".chip-label")?.textContent ?? "",
      c.querySelector(".chip-value")?.textContent ?? "",
    ]);
  const popover = () => r.getByRole("dialog");
  const input = (name: string) => r.getByRole("combobox", { name }) as HTMLInputElement;
  const valueBox = () => r.getByRole("textbox", { name: "Value" }) as HTMLInputElement;
  const set = (el: HTMLInputElement, v: string) => {
    el.value = v;
    fireEvent.input(el);
  };
  return { ...r, value, writes, chips, popover, input, valueBox, set };
}

const TWO: Json = [
  { number: "+61400000020", label: "mobile" },
  { number: "+61400000021", label: "home" },
  { number: "+61400000020", label: "work" },
];

describe("which lists are labelled chips", () => {
  it("is a list of dicts of exactly a text value key and a text label key, with no other entries", () => {
    expect(labelledShape(PHONES)).toEqual({ key: "number", labelRequired: false });
    // Key order does not matter, and a required label is written even when empty.
    expect(labelledShape(listOf([def("label", "string", { required: true }), def("value", "string")]))).toEqual({ key: "value", labelRequired: true });
  });

  it("is not any other list or dict", () => {
    const str = (n: string) => def(n, "string");
    expect(labelledShape(def("tags", "list", { items: "string", item: str("tags") }))).toBeUndefined();
    expect(labelledShape(def("phones", "dict", { keys: [str("number"), str("label")], values: null }))).toBeUndefined();
    expect(labelledShape(listOf([str("number"), str("kind")]))).toBeUndefined();
    expect(labelledShape(listOf([str("label")]))).toBeUndefined();
    expect(labelledShape(listOf([str("number"), str("label"), str("note")]))).toBeUndefined();
    expect(labelledShape(listOf([def("number", "int"), str("label")]))).toBeUndefined();
    expect(labelledShape(listOf([str("number"), def("label", "select", { options: ["a"] })]))).toBeUndefined();
    expect(labelledShape(listOf([str("number"), str("label")], "any"))).toBeUndefined();
    expect(labelledShape(listOf([str("number"), str("label")], str("other")))).toBeUndefined();
    // An older server's list with no item definition.
    expect(labelledShape(def("phones", "list", { items: "dict" }))).toBeUndefined();
  });

  it("leaves another list of dicts on the generic list editor", () => {
    const f = listOf([def("number", "string"), def("kind", "string")]);
    const r = render(() => <FieldInput field={f} value={[{ number: "1", kind: "x" }]} onChange={() => {}} label="Phones" id="p" />);
    expect(r.container.querySelector(".labelled-chip")).toBeNull();
    expect(r.getAllByTestId("list-item")).toHaveLength(1);
  });
});

describe("labelled list chips", () => {
  afterEach(() => vi.mocked(copyText).mockClear());

  it("renders one chip per value with its label as a prefix", () => {
    const m = mount([{ number: "+61400000020", label: "mobile" }, { number: "+61400000021" }]);
    expect(m.chips()).toEqual([
      ["mobile", "+61400000020"],
      ["", "+61400000021"],
    ]);
    // No label, no prefix.
    expect(m.container.querySelectorAll(".chip-label")).toHaveLength(1);
  });

  it("copies the value, not the label, and says so", async () => {
    const m = mount(TWO);
    fireEvent.click(m.getByRole("button", { name: "Copy home +61400000021" }));
    await waitFor(() => expect(copyText).toHaveBeenCalledWith("+61400000021"));
    await waitFor(() => expect(m.getByRole("button", { name: "Copied +61400000021" })).toBeTruthy());
    expect(m.getByRole("status").textContent).toBe("Copied");
  });

  it("edits the label and value of one index and writes the list of records", async () => {
    const m = mount(TWO);
    // Clicking the chip text opens Edit for that chip: the third, a duplicate of the first.
    fireEvent.click(m.getByRole("button", { name: "work +61400000020" }));
    await waitFor(() => expect(m.valueBox().value).toBe("+61400000020"));
    expect(m.input("Label").value).toBe("work");
    m.set(m.input("Label"), "office");
    m.set(m.valueBox(), "+61299990000");
    fireEvent.keyDown(m.valueBox(), { key: "Enter" });
    expect(m.writes).toEqual([
      [
        { number: "+61400000020", label: "mobile" },
        { number: "+61400000021", label: "home" },
        { number: "+61299990000", label: "office" },
      ],
    ]);
    expect(m.queryByRole("dialog")).toBeNull();
  });

  it("writes no label member for a value whose label is cleared", async () => {
    const m = mount(TWO);
    fireEvent.click(m.getByRole("button", { name: "home +61400000021" }));
    await waitFor(() => expect(m.input("Label").value).toBe("home"));
    m.set(m.input("Label"), "  ");
    fireEvent.click(m.getByRole("button", { name: "Save" }));
    expect(m.writes).toEqual([
      [
        { number: "+61400000020", label: "mobile" },
        { number: "+61400000021" },
        { number: "+61400000020", label: "work" },
      ],
    ]);
    expect(m.chips()[1]).toEqual(["", "+61400000021"]);
  });

  it("cancels an edit with Escape and writes nothing", async () => {
    const m = mount(TWO);
    fireEvent.click(m.getByRole("button", { name: "Edit mobile +61400000020" }));
    await waitFor(() => expect(m.valueBox().value).toBe("+61400000020"));
    m.set(m.valueBox(), "changed");
    fireEvent.keyDown(m.valueBox(), { key: "Escape" });
    expect(m.queryByRole("dialog")).toBeNull();
    expect(m.writes).toEqual([]);
  });

  it("adds a record to the end, offering the usual labels and those other notes use", async () => {
    const m = mount(TWO);
    fireEvent.click(m.getByRole("button", { name: "Add to Phones" }));
    await waitFor(() => expect(m.popover()).toBeTruthy());
    const options = () => [...m.container.querySelectorAll("datalist option")].map((o) => o.getAttribute("value"));
    // "pager" is the label another person's phone record holds.
    await waitFor(() => expect(options()).toContain("pager"));
    expect(options().slice(0, 5)).toEqual(["mobile", "home", "work", "main", "other"]);
    m.set(m.input("Label"), "main");
    m.set(m.valueBox(), "+61400000000");
    fireEvent.click(m.getByRole("button", { name: "Save" }));
    expect(m.writes).toEqual([[...(TWO as Json[]), { number: "+61400000000", label: "main" }]]);
    expect(m.chips().at(-1)).toEqual(["main", "+61400000000"]);
  });

  it("adds the first record to an empty field", async () => {
    const m = mount(undefined);
    fireEvent.click(m.getByRole("button", { name: "Add to Phones" }));
    await waitFor(() => expect(m.popover()).toBeTruthy());
    m.set(m.valueBox(), "+61400000000");
    fireEvent.click(m.getByRole("button", { name: "Save" }));
    expect(m.writes).toEqual([[{ number: "+61400000000" }]]);
  });

  it("removes the same index, even with duplicate values", () => {
    const m = mount(TWO);
    fireEvent.click(m.getByRole("button", { name: "Remove work +61400000020" }));
    expect(m.writes).toEqual([
      [
        { number: "+61400000020", label: "mobile" },
        { number: "+61400000021", label: "home" },
      ],
    ]);
    expect(m.chips()).toEqual([
      ["mobile", "+61400000020"],
      ["home", "+61400000021"],
    ]);
  });

  it("removes the field when the last value goes", () => {
    const m = mount([{ number: "+61400000020", label: "mobile" }]);
    fireEvent.click(m.getByRole("button", { name: "Remove mobile +61400000020" }));
    expect(m.writes).toEqual([undefined]);
  });

  it("works by index in its helpers, and reads as label-value pairs", () => {
    const l = labelled([{ v: "a", label: "x" }, { v: "b" }, "a"], "v");
    expect(l).toEqual({ values: ["a", "b", "a"], labels: ["x", "", ""] });
    expect(editAt(l, 2, "c", "y")).toEqual({ values: ["a", "b", "c"], labels: ["x", "", "y"] });
    expect(removeAt(l, 0)).toEqual({ values: ["b", "a"], labels: ["", ""] });
    expect(addTo(l, "d", "")).toEqual({ values: ["a", "b", "a", "d"], labels: ["x", "", "", ""] });
    expect(labelledValue(labelled([{ v: "a" }], "v"), { key: "v", labelRequired: false })).toEqual([{ v: "a" }]);
    expect(labelledValue(labelled([{ v: "a" }], "v"), { key: "v", labelRequired: true })).toEqual([{ v: "a", label: "" }]);
    expect(labelledValue(labelled([], "v"), { key: "v", labelRequired: false })).toBeUndefined();
    expect(labelledText(labelled(TWO, "number"))).toBe("mobile +61400000020, home +61400000021, work +61400000020");
    expect(fieldText(PHONES, (TWO as Json[]).slice(0, 2))).toBe("mobile +61400000020, home +61400000021");
  });
});
