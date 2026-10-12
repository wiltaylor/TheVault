import { describe, expect, it } from "vitest";
import { datetimeInput, datetimeValue, scalarValue, termOptions } from "./FieldInput";

describe("field inputs", () => {
  it("parses input text into each scalar's encoding", () => {
    expect(scalarValue("int", "4.7")).toBe(4);
    expect(scalarValue("float", "4.5")).toBe(4.5);
    expect(scalarValue("int", "")).toBeUndefined();
    expect(scalarValue("date", "2026-11-01")).toBe("2026-11-01");
    expect(scalarValue("datetime", "2026-10-20T09:30", "2026-01-01T00:00:00+11:00")).toBe("2026-10-20T09:30:00+11:00");
  });

  it("shows a stored datetime's local part and keeps its offset", () => {
    expect(datetimeInput("2026-10-20T09:30:00.250-05:00")).toBe("2026-10-20T09:30:00");
    expect(datetimeInput(3)).toBe("");
    expect(datetimeValue("2026-10-20T09:30:15", "2026-10-20T09:30:00Z")).toBe("2026-10-20T09:30:15Z");
    expect(datetimeValue("2026-10-20T09:30", undefined)).toMatch(/^2026-10-20T09:30:00(Z|[+-]\d\d:\d\d)$/);
  });

  it("lists terms parents first with their depth", () => {
    const leaf = { name: "Blocked", path: "Active/Blocked", colour: null, description: null, children: [] };
    const terms = [{ name: "Active", path: "Active", colour: null, description: null, children: [leaf] }];
    expect(termOptions(terms)).toEqual([
      { value: "Active", text: "Active", depth: 0 },
      { value: "Active/Blocked", text: "Blocked", depth: 1 },
    ]);
  });
});
