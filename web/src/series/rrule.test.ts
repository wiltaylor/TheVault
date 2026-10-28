import { describe, expect, it } from "vitest";
import type { FieldDef } from "../api";
import { type RuleModel, defaultRule, isRuleField, modelOf, modelSummary, readRule, ruleLine, ruleSummary } from "./rrule";

const model = (more: Partial<RuleModel>): RuleModel => ({ ...defaultRule(), ...more });

describe("the rule builder's model", () => {
  const cases: [string, RuleModel][] = [
    ["RRULE:FREQ=DAILY", model({ freq: "daily" })],
    ["RRULE:FREQ=DAILY;INTERVAL=3;COUNT=5", model({ freq: "daily", interval: 3, end: { kind: "count", count: 5 } })],
    ["RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE", model({ interval: 2, weekdays: ["MO", "WE"] })],
    ["RRULE:FREQ=MONTHLY;BYMONTHDAY=15", model({ freq: "monthly", monthly: { by: "day", day: 15 } })],
    ["RRULE:FREQ=MONTHLY;BYMONTHDAY=-1", model({ freq: "monthly", monthly: { by: "day", day: -1 } })],
    ["RRULE:FREQ=MONTHLY;BYDAY=2TU", model({ freq: "monthly", monthly: { by: "weekday", nth: 2, weekday: "TU" } })],
    ["RRULE:FREQ=MONTHLY;BYDAY=-1FR", model({ freq: "monthly", monthly: { by: "weekday", nth: -1, weekday: "FR" } })],
    ["RRULE:FREQ=YEARLY", model({ freq: "yearly" })],
    ["RRULE:FREQ=WEEKLY;UNTIL=20270630", model({ end: { kind: "until", date: "2027-06-30" } })],
  ];

  it("reads each RRULE line the builder offers, and writes it back the same", () => {
    for (const [line, m] of cases) {
      expect(modelOf(line), line).toEqual(m);
      expect(ruleLine(m, true), line).toBe(line);
    }
  });

  it("round-trips a timed series' end date through a UTC UNTIL", () => {
    const m = model({ weekdays: ["TU"], end: { kind: "until", date: "2027-06-30" } });
    const line = ruleLine(m, false);
    expect(line).toMatch(/^RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=\d{8}T\d{6}Z$/);
    expect(modelOf(line)).toEqual(m);
  });

  it("writes weekdays in week order and leaves out an interval of 1", () => {
    expect(ruleLine(model({ weekdays: ["FR", "MO"] }), false)).toBe("RRULE:FREQ=WEEKLY;BYDAY=MO,FR");
    expect(ruleLine(model({ interval: 1 }), false)).toBe("RRULE:FREQ=WEEKLY");
  });
});

describe("raw mode", () => {
  it("opens a plain RRULE in the builder", () => {
    expect(readRule(["RRULE:FREQ=WEEKLY;BYDAY=MO"])).toEqual({ model: model({ weekdays: ["MO"] }), reason: null });
    expect(readRule([])).toEqual({ model: null, reason: null });
  });

  it("shows EXDATE and RDATE lines, several RRULEs and parts it does not offer as raw lines, saying why", () => {
    const raw = (lines: string[]) => readRule(lines).reason;
    expect(raw(["RRULE:FREQ=WEEKLY", "EXDATE;TZID=Australia/Melbourne:20261109T100000"])).toContain("EXDATE");
    expect(raw(["RRULE:FREQ=WEEKLY", "RDATE:20261109"])).toContain("RDATE");
    expect(raw(["RRULE:FREQ=WEEKLY", "RRULE:FREQ=MONTHLY"])).toContain("more than one RRULE");
    expect(raw(["RRULE:FREQ=MONTHLY;BYDAY=TU;BYSETPOS=2"])).toContain("BYSETPOS");
    expect(raw(["RRULE:FREQ=HOURLY"])).toContain("hourly");
    expect(raw(["RRULE:FREQ=MONTHLY;BYDAY=MO,TU"])).toContain("BYDAY");
    expect(raw(["RRULE:FREQ=YEARLY;BYMONTH=6"])).toContain("BYMONTH");
    expect(raw(["RRULE:FREQ=WEEKLY;COUNT=2;UNTIL=20270101"])).toContain("both COUNT and UNTIL");
  });
});

describe("the rule's summary", () => {
  it("says what the builder's rules do", () => {
    expect(modelSummary(model({ interval: 2, weekdays: ["MO", "WE"], end: { kind: "until", date: "2027-06-30" } }))).toBe(
      "Every 2 weeks on Mon and Wed, until 30 June 2027",
    );
    expect(ruleSummary(["RRULE:FREQ=DAILY"])).toBe("Every day");
    expect(ruleSummary(["RRULE:FREQ=DAILY;INTERVAL=3;COUNT=1"])).toBe("Every 3 days, once");
    expect(ruleSummary(["RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE;COUNT=10"])).toBe("Every week on Mon, Tue and Wed, 10 times");
    expect(ruleSummary(["RRULE:FREQ=MONTHLY;BYDAY=2TU"])).toBe("Every month on the 2nd Tuesday");
    expect(ruleSummary(["RRULE:FREQ=MONTHLY;BYDAY=-1FR"])).toBe("Every month on the last Friday");
    expect(ruleSummary(["RRULE:FREQ=MONTHLY;BYMONTHDAY=15"])).toBe("Every month on day 15");
    expect(ruleSummary(["RRULE:FREQ=YEARLY;INTERVAL=2"])).toBe("Every 2 years");
  });

  it("describes raw rules too: exception and extra dates, and custom rules", () => {
    expect(ruleSummary(["RRULE:FREQ=WEEKLY;BYDAY=MO", "EXDATE;TZID=Australia/Melbourne:20261109T100000"])).toBe(
      "Every week on Mon, except 1 date",
    );
    expect(ruleSummary(["RRULE:FREQ=WEEKLY", "EXDATE:20261109T000000Z,20261116T000000Z", "RDATE;VALUE=DATE:20261201"])).toBe(
      "Every week, except 2 dates, plus 1 date",
    );
    expect(ruleSummary(["RRULE:FREQ=MONTHLY;BYDAY=TU;BYSETPOS=2"])).toBe("Custom rule");
    expect(ruleSummary([])).toBe("");
  });
});

describe("which fields are a series' rule", () => {
  const def = (more: Partial<FieldDef>): FieldDef => ({
    name: "rule",
    label: "Repeats",
    description: null,
    kind: "list",
    items: "string",
    required: true,
    computed: false,
    multiple: false,
    compound: false,
    ...more,
  });

  it("is a list of strings named rule on an event series", () => {
    expect(isRuleField(def({}), "event_series")).toBe(true);
    expect(isRuleField(def({ name: "rules" }), "event_series")).toBe(false);
    expect(isRuleField(def({ items: "int" }), "event_series")).toBe(false);
    // Without the series type loaded, a rule on another type is just a list.
    expect(isRuleField(def({}), "project")).toBe(false);
  });
});
