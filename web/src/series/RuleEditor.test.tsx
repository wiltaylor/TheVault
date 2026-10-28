import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FieldDef, Json, TypeDef } from "../api";

const types: TypeDef[] = [];
vi.mock("../session", async (orig) => {
  const real = await orig<typeof import("../session")>();
  return { ...real, typeDef: (name: string) => types.find((t) => t.name === name) };
});

const { FieldInput } = await import("../components/FieldInput");
const { fieldText } = await import("../components/Render");

afterEach(cleanup);

const RULE: FieldDef = {
  name: "rule",
  label: "Repeats",
  description: null,
  kind: "list",
  items: "string",
  required: true,
  computed: false,
  multiple: false,
  compound: false,
};

function mount(initial: Json | undefined) {
  const [value, setValue] = createSignal<Json | undefined>(initial);
  const m = render(() => (
    <FieldInput field={RULE} value={value()} onChange={setValue} label="Repeats" id="rule" related={{ type: "event_series" }} />
  ));
  return { m, value };
}

describe("the rule editor", () => {
  it("builds a weekly rule from the frequency and the days, with its summary", () => {
    const { m, value } = mount(undefined);
    fireEvent.change(m.getByLabelText("Repeats", { selector: "select" }), { target: { value: "weekly" } });
    expect(value()).toEqual(["RRULE:FREQ=WEEKLY"]);
    fireEvent.click(m.getByRole("button", { name: "Monday" }));
    fireEvent.click(m.getByRole("button", { name: "Wednesday" }));
    expect(value()).toEqual(["RRULE:FREQ=WEEKLY;BYDAY=MO,WE"]);
    fireEvent.change(m.getByLabelText("every"), { target: { value: "2" } });
    expect(value()).toEqual(["RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE"]);
    expect(m.getByTestId("rule-summary").textContent).toBe("Every 2 weeks on Mon and Wed");
    expect(m.getByRole("button", { name: "Monday" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("sets a monthly rule on the nth weekday and an end after a number of times", () => {
    const { m, value } = mount(["RRULE:FREQ=MONTHLY"]);
    fireEvent.change(m.getByLabelText("On"), { target: { value: "weekday" } });
    fireEvent.change(m.getByLabelText("Which"), { target: { value: "2" } });
    fireEvent.change(m.getByLabelText("Weekday"), { target: { value: "TU" } });
    expect(value()).toEqual(["RRULE:FREQ=MONTHLY;BYDAY=2TU"]);
    fireEvent.change(m.getByLabelText("Ends"), { target: { value: "count" } });
    fireEvent.change(m.getByLabelText("Times"), { target: { value: "6" } });
    expect(value()).toEqual(["RRULE:FREQ=MONTHLY;BYDAY=2TU;COUNT=6"]);
    expect(m.getByTestId("rule-summary").textContent).toBe("Every month on the 2nd Tuesday, 6 times");
  });

  it("opens a rule the builder cannot show as raw lines, with a note, and edits the lines", () => {
    const { m, value } = mount(["RRULE:FREQ=WEEKLY;BYDAY=MO", "EXDATE:20261109T000000Z"]);
    expect(m.getByTestId("rule-raw-note").textContent).toContain("EXDATE");
    expect(m.queryByLabelText("Repeats", { selector: "select" })).toBeNull();
    const area = m.getByLabelText("RRULE lines") as HTMLTextAreaElement;
    expect(area.value).toBe("RRULE:FREQ=WEEKLY;BYDAY=MO\nEXDATE:20261109T000000Z");
    expect(m.getByTestId("rule-summary").textContent).toBe("Every week on Mon, except 1 date");
    fireEvent.input(area, { target: { value: "RRULE:FREQ=DAILY\n\n" } });
    expect(value()).toEqual(["RRULE:FREQ=DAILY"]);
    // Back to a rule the builder shows.
    expect(m.queryByTestId("rule-raw-note")).toBeNull();
  });

  it("reads as its summary in cards and tables, and another type's list named rule does not", () => {
    const value = ["RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;UNTIL=20270630"];
    expect(fieldText(RULE, value)).toBe("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;UNTIL=20270630");
    types.push({ name: "event_series", label: "Event series", fields: [RULE] } as unknown as TypeDef);
    expect(fieldText(RULE, value)).toBe("Every 2 weeks on Mon and Wed, until 30 June 2027");
    expect(fieldText({ ...RULE, label: "Rules of the game" }, value)).toBe(value[0]);
    types.length = 0;
  });
});
