import { afterEach, describe, expect, it } from "vitest";
import {
  PERIOD_KEY,
  isoDay,
  isoWeek,
  lastPeriod,
  longDay,
  periodDays,
  periodEnd,
  periodOfType,
  periodStart,
  periodTitle,
  plannerHref,
  plannerOfLocation,
  rememberPeriod,
  shiftDay,
  shiftPeriod,
} from "./period";

describe("planner days", () => {
  it("writes a day in the browser's own time zone", () => {
    expect(isoDay(new Date(2026, 9, 13, 23, 59))).toBe("2026-10-13");
    expect(isoDay(new Date(2026, 0, 1, 0, 0))).toBe("2026-01-01");
  });

  it("steps across month, year and leap-day boundaries", () => {
    expect(shiftDay("2026-10-13", 1)).toBe("2026-10-14");
    expect(shiftDay("2026-10-31", 1)).toBe("2026-11-01");
    expect(shiftDay("2026-01-01", -1)).toBe("2025-12-31");
    expect(shiftDay("2024-02-28", 1)).toBe("2024-02-29");
    expect(shiftDay("2026-03-01", -1)).toBe("2026-02-28");
  });

  it("writes a day out the way the server titles daily notes", () => {
    expect(longDay("2026-10-13")).toBe("Tuesday 13 October 2026");
    expect(longDay("2024-02-29")).toBe("Thursday 29 February 2024");
    expect(longDay("2000-01-01")).toBe("Saturday 1 January 2000");
  });
});

describe("ISO weeks", () => {
  it("numbers weeks from the week holding the year's first Thursday", () => {
    expect(isoWeek("2026-10-25")).toEqual({ year: 2026, week: 43 });
    expect(isoWeek("2026-10-19")).toEqual({ year: 2026, week: 43 });
    expect(isoWeek("2026-01-01")).toEqual({ year: 2026, week: 1 });
  });

  it("puts early January in the last week of the year before", () => {
    // 2021-01-03 is a Sunday; that week's Thursday is 31 December 2020.
    expect(isoWeek("2021-01-03")).toEqual({ year: 2020, week: 53 });
    expect(isoWeek("2027-01-01")).toEqual({ year: 2026, week: 53 });
    expect(isoWeek("2023-01-01")).toEqual({ year: 2022, week: 52 });
  });

  it("puts late December in week 1 of the year after", () => {
    expect(isoWeek("2024-12-30")).toEqual({ year: 2025, week: 1 });
    expect(isoWeek("2025-12-31")).toEqual({ year: 2026, week: 1 });
    expect(isoWeek("2026-12-31")).toEqual({ year: 2026, week: 53 });
  });
});

describe("periods", () => {
  it("starts a week on its Monday, a quarter on its first month", () => {
    expect(periodStart("week", "2026-10-25")).toBe("2026-10-19");
    expect(periodStart("week", "2026-10-19")).toBe("2026-10-19");
    expect(periodStart("week", "2027-01-01")).toBe("2026-12-28");
    expect(periodStart("month", "2026-10-25")).toBe("2026-10-01");
    expect(periodStart("quarter", "2026-12-31")).toBe("2026-10-01");
    expect(periodStart("quarter", "2026-03-31")).toBe("2026-01-01");
    expect(periodStart("year", "2026-10-25")).toBe("2026-01-01");
    expect(periodStart("day", "2026-10-25")).toBe("2026-10-25");
  });

  it("ends each period on its last day", () => {
    expect(periodEnd("week", "2026-10-25")).toBe("2026-10-25");
    expect(periodEnd("month", "2024-02-10")).toBe("2024-02-29");
    expect(periodEnd("quarter", "2026-10-25")).toBe("2026-12-31");
    expect(periodEnd("year", "2026-10-25")).toBe("2026-12-31");
    expect(periodDays("week", "2026-10-21")).toHaveLength(7);
    expect(periodDays("quarter", "2026-01-05")).toHaveLength(90);
  });

  it("steps a period at a time from any day in it", () => {
    expect(shiftPeriod("week", "2026-10-25", 1)).toBe("2026-10-26");
    expect(shiftPeriod("week", "2026-01-01", -1)).toBe("2025-12-22");
    expect(shiftPeriod("month", "2026-01-31", 1)).toBe("2026-02-01");
    expect(shiftPeriod("month", "2026-01-15", -1)).toBe("2025-12-01");
    expect(shiftPeriod("quarter", "2026-11-15", 1)).toBe("2027-01-01");
    expect(shiftPeriod("year", "2026-11-15", -1)).toBe("2025-01-01");
    expect(shiftPeriod("day", "2026-12-31", 1)).toBe("2027-01-01");
  });

  it("titles each period as the server does", () => {
    expect(periodTitle("day", "2026-10-13")).toBe("Tuesday 13 October 2026");
    expect(periodTitle("week", "2026-10-19")).toBe("Week 43, 2026");
    expect(periodTitle("week", "2024-12-30")).toBe("Week 1, 2025");
    expect(periodTitle("month", "2026-10-01")).toBe("October 2026");
    expect(periodTitle("quarter", "2026-10-01")).toBe("Q4 2026");
    expect(periodTitle("year", "2026-01-01")).toBe("2026");
  });

  it("knows each period's type", () => {
    expect(periodOfType("weeklynote")).toBe("week");
    expect(periodOfType("dailynote")).toBe("day");
    expect(periodOfType("task")).toBeUndefined();
  });
});

describe("planner locations", () => {
  afterEach(() => localStorage.clear());

  it("links the current period without its date", () => {
    expect(plannerHref("week", "2026-10-21", "2026-10-25")).toBe("/planner/week");
    expect(plannerHref("week", "2026-10-28", "2026-10-25")).toBe("/planner/week/2026-10-26");
    expect(plannerHref("day", "2026-10-24", "2026-10-25")).toBe("/planner/day/2026-10-24");
  });

  it("reads the period and day a location shows", () => {
    expect(plannerOfLocation("/planner/week", "2026-10-25")).toEqual({ period: "week", day: "2026-10-19" });
    expect(plannerOfLocation("/planner/month/2026-03-14?x#y", "2026-10-25")).toEqual({ period: "month", day: "2026-03-01" });
    expect(plannerOfLocation("/daily", "2026-10-25")).toEqual({ period: "day", day: "2026-10-25" });
    expect(plannerOfLocation("/daily/2026-10-05", "2026-10-25")).toEqual({ period: "day", day: "2026-10-05" });
    expect(plannerOfLocation("/planner/decade", "2026-10-25")).toBeUndefined();
    expect(plannerOfLocation("/notes/abc", "2026-10-25")).toBeUndefined();
    expect(plannerOfLocation("/dailyish", "2026-10-25")).toBeUndefined();
  });

  it("remembers the last period chosen", () => {
    expect(lastPeriod()).toBe("day");
    expect(plannerOfLocation("/planner", "2026-10-25")).toEqual({ period: "day", day: "2026-10-25" });
    rememberPeriod("quarter");
    expect(localStorage.getItem(PERIOD_KEY)).toBe("quarter");
    expect(lastPeriod()).toBe("quarter");
    expect(plannerOfLocation("/planner", "2026-10-25")).toEqual({ period: "quarter", day: "2026-10-01" });
  });
});
