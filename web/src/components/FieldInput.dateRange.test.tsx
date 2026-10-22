import { fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import type { FieldDef, Json } from "../api";
import { FieldInput, datetimeValue, rangeIsAllDay, switchAllDay } from "./FieldInput";

const WHEN: FieldDef = {
  name: "when",
  label: "When",
  description: null,
  kind: "date_range",
  of: "date_or_datetime",
  required: false,
  computed: false,
  multiple: false,
  compound: false,
};

function mount(start?: Json, field: FieldDef = WHEN) {
  const [value, setValue] = createSignal<Json | undefined>(start);
  const r = render(() => <FieldInput field={field} value={value()} onChange={setValue} label={field.label} id="when" />);
  const input = (name: string) => r.getByLabelText(name, { selector: "input" }) as HTMLInputElement;
  return { ...r, value, setValue, input };
}

const type = (el: HTMLInputElement, text: string) => fireEvent.input(el, { target: { value: text } });

describe("date or datetime range input", () => {
  it("edits an all-day range with date inputs and a timed one with datetime inputs", () => {
    const dated = mount({ start: "2026-10-20", end: "2026-10-21" });
    expect((dated.getByLabelText("All day") as HTMLInputElement).checked).toBe(true);
    expect(dated.input("Start").type).toBe("date");
    expect(dated.input("End").value).toBe("2026-10-21");
    type(dated.input("End"), "2026-10-22");
    expect(dated.value()).toEqual({ start: "2026-10-20", end: "2026-10-22" });
    dated.unmount();

    const timed = mount({ start: "2026-10-20T09:30:00+11:00", end: "2026-10-20T10:00:00+11:00" });
    expect((timed.getByLabelText("All day") as HTMLInputElement).checked).toBe(false);
    expect(timed.input("Start").type).toBe("datetime-local");
    expect(timed.input("Start").value).toBe("2026-10-20T09:30");
    type(timed.input("End"), "2026-10-20T11:15");
    expect(timed.value()).toEqual({ start: "2026-10-20T09:30:00+11:00", end: "2026-10-20T11:15:00+11:00" });
  });

  it("turning All day off makes the days 00:00 to 23:59, and on again keeps the dates", () => {
    const m = mount({ start: "2026-10-20", end: "2026-10-21" });
    fireEvent.click(m.getByLabelText("All day"));
    expect(m.input("Start").type).toBe("datetime-local");
    expect(m.value()).toEqual({
      start: datetimeValue("2026-10-20T00:00", undefined),
      end: datetimeValue("2026-10-21T23:59", undefined),
    });
    expect(m.input("End").value).toBe("2026-10-21T23:59");
    fireEvent.click(m.getByLabelText("All day"));
    expect(m.input("Start").type).toBe("date");
    expect(m.value()).toEqual({ start: "2026-10-20", end: "2026-10-21" });
  });

  it("an empty range starts all day, and a timed range written elsewhere switches the inputs", () => {
    const m = mount(undefined);
    expect((m.getByLabelText("All day") as HTMLInputElement).checked).toBe(true);
    type(m.input("Start"), "2026-10-20");
    expect(m.value()).toBeUndefined();
    fireEvent.click(m.getByLabelText("All day"));
    expect(m.input("Start").value).toBe("2026-10-20T00:00");
    m.setValue({ start: "2026-11-01", end: "2026-11-01" });
    expect((m.getByLabelText("All day") as HTMLInputElement).checked).toBe(true);
    expect(m.input("End").value).toBe("2026-11-01");
  });

  it("a date or a datetime range has no All day box", () => {
    const m = mount({ start: "2026-10-20", end: "2026-10-21" }, { ...WHEN, of: "date" });
    expect(m.queryByLabelText("All day")).toBeNull();
    expect(m.input("Start").type).toBe("date");
  });

  it("converts each end by its form", () => {
    expect(rangeIsAllDay({})).toBe(true);
    expect(rangeIsAllDay({ start: "2026-10-20T09:00:00Z" })).toBe(false);
    expect(switchAllDay({ start: "2026-10-20T23:30:00-05:00", end: "2026-10-21T01:00:00-05:00" }, true)).toEqual({
      start: "2026-10-20",
      end: "2026-10-21",
    });
    expect(switchAllDay({ start: "2026-10-20T09:00:00Z", end: undefined }, false)).toEqual({
      start: "2026-10-20T09:00:00Z",
      end: undefined,
    });
  });
});
