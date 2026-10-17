import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FieldDef, Json } from "../api";

vi.mock("../clipboard", () => ({ copyText: vi.fn(async () => true) }));
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    api: { ...real.api, query: vi.fn(async () => ({ rows: [{ id: "x", type: "person", title: "X", fields: { phone_labels: ["pager"] } }] })) },
  };
});

import { copyText } from "../clipboard";
import { FieldInput, type RelatedFields } from "./FieldInput";
import { addTo, editAt, labelled, labelledText, labelledWrite, removeAt } from "./LabelledList";
import { fieldText } from "./Render";

const PHONES: FieldDef = {
  name: "phones",
  label: "Phones",
  description: null,
  kind: "list",
  items: "string",
  labels: "phone_labels",
  required: false,
  computed: false,
  multiple: false,
  compound: false,
};

/** A person note's fields in a signal, edited through FieldInput as the note page does. */
function mount(start: Record<string, Json>) {
  const [fields, setFields] = createSignal<Record<string, Json | undefined>>(start);
  const writes: Record<string, Json | undefined>[] = [];
  const related: RelatedFields = {
    type: "person",
    value: (n) => fields()[n],
    set: (values) => {
      writes.push(values);
      setFields({ ...fields(), ...values });
    },
  };
  const r = render(() => (
    <FieldInput field={PHONES} value={fields().phones} related={related} onChange={() => {}} label="Phones" id="phones" />
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
  return { ...r, fields, writes, chips, popover, input, valueBox, set };
}

const TWO = {
  phones: ["+61400000020", "+61400000021", "+61400000020"],
  phone_labels: ["mobile", "home", "work"],
};

describe("labelled list chips", () => {
  afterEach(() => vi.mocked(copyText).mockClear());

  it("renders one chip per value with its label as a prefix", () => {
    const m = mount({ phones: ["+61400000020", "+61400000021"], phone_labels: ["mobile"] });
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

  it("edits the label and value of one index and writes both lists in one write", async () => {
    const m = mount(TWO);
    // Clicking the chip text opens Edit for that chip: the third, a duplicate of the first.
    fireEvent.click(m.getByRole("button", { name: "work +61400000020" }));
    await waitFor(() => expect(m.valueBox().value).toBe("+61400000020"));
    expect(m.input("Label").value).toBe("work");
    m.set(m.input("Label"), "office");
    m.set(m.valueBox(), "+61299990000");
    fireEvent.keyDown(m.valueBox(), { key: "Enter" });
    expect(m.writes).toEqual([
      {
        phones: ["+61400000020", "+61400000021", "+61299990000"],
        phone_labels: ["mobile", "home", "office"],
      },
    ]);
    expect(m.queryByRole("dialog")).toBeNull();
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

  it("adds a value and its label to the end of both lists, offering the usual and known labels", async () => {
    const m = mount(TWO);
    fireEvent.click(m.getByRole("button", { name: "Add to Phones" }));
    await waitFor(() => expect(m.popover()).toBeTruthy());
    const options = () => [...m.container.querySelectorAll("datalist option")].map((o) => o.getAttribute("value"));
    await waitFor(() => expect(options()).toContain("pager"));
    expect(options().slice(0, 5)).toEqual(["mobile", "home", "work", "main", "other"]);
    m.set(m.input("Label"), "main");
    m.set(m.valueBox(), "+61400000000");
    fireEvent.click(m.getByRole("button", { name: "Save" }));
    expect(m.writes).toEqual([
      {
        phones: [...TWO.phones, "+61400000000"],
        phone_labels: [...TWO.phone_labels, "main"],
      },
    ]);
    expect(m.chips().at(-1)).toEqual(["main", "+61400000000"]);
  });

  it("removes the same index from both lists, even with duplicate values", () => {
    const m = mount(TWO);
    fireEvent.click(m.getByRole("button", { name: "Remove work +61400000020" }));
    expect(m.writes).toEqual([{ phones: ["+61400000020", "+61400000021"], phone_labels: ["mobile", "home"] }]);
    expect(m.chips()).toEqual([
      ["mobile", "+61400000020"],
      ["home", "+61400000021"],
    ]);
  });

  it("removes both fields when the last value goes", () => {
    const m = mount({ phones: ["+61400000020"], phone_labels: ["mobile"] });
    fireEvent.click(m.getByRole("button", { name: "Remove mobile +61400000020" }));
    expect(m.writes).toEqual([{ phones: undefined, phone_labels: undefined }]);
  });

  it("works by index in its helpers, and reads as label-value pairs", () => {
    const l = labelled(["a", "b", "a"], ["x"]);
    expect(l).toEqual({ values: ["a", "b", "a"], labels: ["x", "", ""] });
    expect(editAt(l, 2, "c", "y")).toEqual({ values: ["a", "b", "c"], labels: ["x", "", "y"] });
    expect(removeAt(l, 0)).toEqual({ values: ["b", "a"], labels: ["", ""] });
    expect(addTo(l, "d", "")).toEqual({ values: ["a", "b", "a", "d"], labels: ["x", "", "", ""] });
    expect(labelledWrite(labelled(["a"], []), "v", "l")).toEqual({ v: ["a"], l: undefined });
    expect(labelledText(labelled(["+61400000020", "+61400000021"], ["mobile", "home"]))).toBe(
      "mobile +61400000020, home +61400000021",
    );
    expect(fieldText(PHONES, TWO.phones.slice(0, 2), TWO)).toBe("mobile +61400000020, home +61400000021");
  });
});
