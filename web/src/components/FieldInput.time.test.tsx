import { fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import type { FieldDef, Json } from "../api";
import { typeFromSchema, typeWcl, takesDefault, problems, type SchemaType } from "../defs/typeModel";
import { filterCond, opsFor } from "../search/searchModel";
import { FieldInput, TIME, anyKind, scalarValue, timeValue } from "./FieldInput";
import { fieldText } from "./Render";

const OPENS: FieldDef = {
  name: "opens",
  label: "Opens",
  description: null,
  kind: "time",
  required: false,
  computed: false,
  multiple: false,
  compound: false,
};

function mount(start?: Json, field: FieldDef = OPENS) {
  const [value, setValue] = createSignal<Json | undefined>(start);
  const r = render(() => <FieldInput field={field} value={value()} onChange={setValue} label={field.label} id="opens" />);
  const input = () => r.getByLabelText(field.label, { selector: "input" }) as HTMLInputElement;
  return { ...r, value, setValue, input };
}

describe("time of day fields", () => {
  it("edit with a time box that writes HH:MM, and HH:MM:SS only when seconds are set", () => {
    const m = mount("09:00");
    expect(m.input().type).toBe("time");
    expect(m.input().value).toBe("09:00");
    expect(m.input().getAttribute("step")).toBeNull();
    fireEvent.input(m.input(), { target: { value: "17:30" } });
    expect(m.value()).toBe("17:30");
    fireEvent.input(m.input(), { target: { value: "17:30:15" } });
    expect(m.value()).toBe("17:30:15");
    // A time with seconds keeps a seconds box.
    expect(m.input().getAttribute("step")).toBe("1");
    fireEvent.input(m.input(), { target: { value: "" } });
    expect(m.value()).toBeUndefined();
  });

  it("read the input's text as a stored time", () => {
    expect(scalarValue("time", "08:05")).toBe("08:05");
    expect(scalarValue("time", "08:05:09")).toBe("08:05:09");
    expect(scalarValue("time", "")).toBeUndefined();
    expect(timeValue("08:05:09.250")).toBe("08:05:09");
    expect(["00:00", "23:59", "12:30:59"].every((t) => TIME.test(t))).toBe(true);
    expect(["24:00", "9:05", "09:5", "09:60", "09:05Z"].some((t) => TIME.test(t))).toBe(false);
    expect(anyKind("09:30")).toBe("time");
  });

  it("show the stored 24-hour time as it is, never am or pm", () => {
    expect(fieldText(OPENS, "17:45")).toBe("17:45");
    expect(fieldText(OPENS, "00:05:30")).toBe("00:05:30");
    const slots: FieldDef = { ...OPENS, name: "slots", kind: "list", items: "time", item: OPENS };
    expect(fieldText(slots, ["13:00", "21:15"])).toBe("13:00, 21:15");
    expect(fieldText(OPENS, "13:00")).not.toMatch(/am|pm/i);
  });

  it("round trip through the type editor as a field, a default, a list item and a dict key", () => {
    const def = (name: string, kind: string, more: Record<string, Json> = {}): Record<string, Json> => ({
      name,
      kind,
      label: name,
      description: null,
      required: false,
      computed: false,
      hidden: false,
      multiple: false,
      default: null,
      ...more,
    });
    const schema: SchemaType = {
      name: "shop",
      label: "Shop",
      plural: "Shops",
      description: null,
      icon: null,
      colour: null,
      fields: [
        def("opens", "time", { default: "09:00" }),
        def("slots", "list", { items: "time", item: def("slots", "time") }),
        def("hours", "dict", { keys: [def("open", "time", { required: true })], values: "time" }),
      ],
      templates: [],
      handlers: [],
    };
    const m = typeFromSchema(schema);
    expect(problems(m)).toEqual([]);
    expect(takesDefault(m.fields[0]!)).toBe(true);
    expect(takesDefault(m.fields[1]!)).toBe(true);
    const text = typeWcl(m);
    expect(text).toContain(`field opens {\n  kind = "time"\n  default = "09:00"\n}\n`);
    expect(text).toContain(`field slots {\n  kind = "list"\n  items = "time"\n}\n`);
    expect(text).toContain(`field hours {\n  kind = "dict"\n  values = "time"\n  key open {\n    kind = "time"\n    required = true\n  }\n}\n`);
  });

  it("filter in search by the clock: at, before, after and between", () => {
    expect(opsFor(OPENS).map(([op, text]) => `${op}:${text}`)).toEqual([
      "on:at",
      "before:before",
      "after:after",
      "between:between",
      "empty:is empty",
      "not_empty:is not empty",
    ]);
    const cond = (op: "on" | "before" | "after" | "between", ...values: string[]) =>
      filterCond(OPENS, { section: "prop", owner: "shop", field: "opens", op, values });
    expect(cond("on", "09:00")).toBe('opens == "09:00"');
    expect(cond("before", "09:00")).toBe('opens < "09:00"');
    expect(cond("after", "17:30:15")).toBe('opens > "17:30:15"');
    expect(cond("between", "12:00", "08:30:00")).toBe('opens >= "08:30:00" && opens <= "12:00"');
    expect(cond("before", "2026-10-20")).toBeNull();
    expect(cond("before", "9:00")).toBeNull();
    const slots: FieldDef = { ...OPENS, name: "slots", kind: "list", items: "time" };
    expect(opsFor(slots)[0]![0]).toBe("has");
  });
});
