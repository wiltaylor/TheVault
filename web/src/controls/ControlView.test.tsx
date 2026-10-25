import { fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import type { Block, FieldDef, Json, TypeDef } from "../api";
import { type ControlBinding, ControlBindingContext } from "./binding";
import { ControlBlock } from "./ControlView";

const field = (name: string, kind: string, extra: Partial<FieldDef> = {}): FieldDef => ({
  name,
  label: name[0]!.toUpperCase() + name.slice(1),
  description: null,
  kind,
  required: false,
  computed: false,
  multiple: false,
  compound: false,
  ...extra,
});

const TYPE: TypeDef = {
  name: "t",
  label: "T",
  plural: "Ts",
  description: null,
  icon: null,
  colour: null,
  fields: [
    field("name", "string", { required: true }),
    field("done", "bool"),
    field("stars", "int"),
    field("words", "int", { computed: true }),
    field("phones", "list", {
      items: "dict",
      item: field("phones", "dict", { keys: [field("number", "string", { required: true }), field("label", "string")], values: null }),
    }),
  ],
  templates: [],
};

const ctl = (f: string, kind: string, extra: Record<string, Json> = {}): Block => ({ kind: "control", fields: { field: f, kind, ...extra } });

/** Render controls bound to a note held in a signal, as the note page binds them. */
function mount(blocks: Block[], start: Record<string, Json> = {}) {
  const [values, setValues] = createSignal<Record<string, Json>>(start);
  const binding: ControlBinding = {
    type: () => TYPE,
    value: (f) => values()[f.name],
    set: (f, v) => {
      const next = { ...values() };
      if (v === undefined) delete next[f.name];
      else next[f.name] = v;
      setValues(next);
    },
  };
  const r = render(() => (
    <ControlBindingContext.Provider value={binding}>
      {blocks.map((b) => (
        <ControlBlock block={b} />
      ))}
    </ControlBindingContext.Provider>
  ));
  return { ...r, values };
}

describe("ControlBlock", () => {
  it("a chips control on a list of labelled values shows each value with its label and writes the records", () => {
    const phones = [
      { number: "+61400000020", label: "mobile" },
      { number: "+61400000021", label: "home" },
    ];
    const { container, getByRole, values } = mount([ctl("phones", "chips")], { phones });
    const chips = [...container.querySelectorAll(".labelled-chip")].map((c) => c.textContent?.replace("×", "").trim());
    expect(chips).toEqual(["mobile+61400000020", "home+61400000021"]);
    fireEvent.click(getByRole("button", { name: "Remove mobile +61400000020" }));
    expect(values().phones).toEqual([{ number: "+61400000021", label: "home" }]);
  });

  it("two controls on one field agree, and each writes the field", () => {
    const { getByRole, values } = mount([ctl("done", "toggle"), ctl("done", "checkbox", { label: "Box" })]);
    const toggle = getByRole("switch", { name: "Done" });
    const box = getByRole("checkbox", { name: "Box" }) as HTMLInputElement;
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(toggle);
    expect(values().done).toBe(true);
    expect(box.checked).toBe(true);
    fireEvent.click(box);
    expect(values().done).toBe(false);
    expect(toggle.getAttribute("aria-checked")).toBe("false");
  });

  it("a rating sets stars, and a click on the current star clears it", () => {
    const { getByRole, values } = mount([ctl("stars", "rating", { max: 4 })]);
    const group = getByRole("radiogroup", { name: "Stars" });
    expect(group.querySelectorAll("[role=radio]")).toHaveLength(4);
    fireEvent.click(getByRole("radio", { name: "3 stars" }));
    expect(values().stars).toBe(3);
    expect(getByRole("radio", { name: "3 stars" }).getAttribute("aria-checked")).toBe("true");
    fireEvent.keyDown(group, { key: "ArrowRight" });
    expect(values().stars).toBe(4);
    fireEvent.click(getByRole("radio", { name: "4 stars" }));
    expect(values().stars).toBeUndefined();
  });

  it("refuses to empty a required field and says why", () => {
    const { getByRole, values } = mount([ctl("name", "text")], { name: "Ada" });
    const box = getByRole("textbox", { name: "Name" }) as HTMLInputElement;
    expect(box.value).toBe("Ada");
    box.value = "";
    fireEvent.input(box);
    expect(values().name).toBe("Ada");
    expect(getByRole("alert").textContent).toBe("Name is required, so it cannot be empty.");
    box.value = "Bo";
    fireEvent.input(box);
    expect(values().name).toBe("Bo");
  });

  it("a control on a computed field is read-only", () => {
    const { getByRole, getByText } = mount([ctl("words", "number")], { words: 12 });
    expect((getByRole("spinbutton") as HTMLInputElement).disabled).toBe(true);
    expect((getByRole("spinbutton") as HTMLInputElement).value).toBe("12");
    expect(getByText(/only the type's handlers write this field/)).toBeTruthy();
  });

  it("with no note around it, a control is disabled and says why", () => {
    const { getByText, container } = render(() => <ControlBlock block={ctl("done", "toggle")} />);
    expect(getByText(/Not on a note/)).toBeTruthy();
    expect(container.querySelector("input")?.disabled).toBe(true);
  });

  it("a control that does not fit its field is shown disabled with the reason", () => {
    const { getByText, container } = mount([ctl("done", "rating")]);
    expect(getByText(/does not fit Done, a bool field/)).toBeTruthy();
    expect(container.querySelector("input")?.disabled).toBe(true);
  });
});
