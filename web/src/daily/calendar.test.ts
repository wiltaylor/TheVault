import { describe, expect, it } from "vitest";
import {
  addDays,
  addMonths,
  dailyHref,
  dailyQuery,
  dayOfLocation,
  daysInMonth,
  gridRange,
  monthGrid,
  monthOf,
  shiftMonth,
  weekStartOf,
  weekdayNames,
} from "./calendar";

describe("month grid", () => {
  it("starts on Monday with the end of the month before, and ends with the start of the next", () => {
    // October 2026 starts on a Thursday and ends on a Saturday.
    const weeks = monthGrid({ year: 2026, month: 10 }, 1);
    expect(weeks).toHaveLength(5);
    expect(weeks.every((w) => w.length === 7)).toBe(true);
    expect(weeks[0]!.map((d) => d.day)).toEqual([
      "2026-09-28",
      "2026-09-29",
      "2026-09-30",
      "2026-10-01",
      "2026-10-02",
      "2026-10-03",
      "2026-10-04",
    ]);
    expect(weeks[0]!.map((d) => d.inMonth)).toEqual([false, false, false, true, true, true, true]);
    expect(weeks[4]!.map((d) => d.date)).toEqual([26, 27, 28, 29, 30, 31, 1]);
    expect(weeks[4]![6]).toEqual({ day: "2026-11-01", date: 1, inMonth: false });
    expect(weeks.flat().filter((d) => d.inMonth)).toHaveLength(31);
  });

  it("starts on Sunday when the week does", () => {
    const weeks = monthGrid({ year: 2026, month: 10 }, 0);
    expect(weeks[0]![0]!.day).toBe("2026-09-27");
    expect(weeks[0]![4]!.day).toBe("2026-10-01");
    expect(weeks[weeks.length - 1]![6]!.day).toBe("2026-10-31");
    expect(weeks).toHaveLength(5);
  });

  it("has no leading days when the 1st is the first day of the week, and up to six weeks", () => {
    // February 2027 starts on a Monday and has exactly four weeks.
    expect(monthGrid({ year: 2027, month: 2 }, 1)).toHaveLength(4);
    expect(monthGrid({ year: 2027, month: 2 }, 1)[0]![0]!.day).toBe("2027-02-01");
    // August 2026 starts on a Saturday: six Monday weeks.
    const aug = monthGrid({ year: 2026, month: 8 }, 1);
    expect(aug).toHaveLength(6);
    expect(gridRange({ year: 2026, month: 8 }, 1)).toEqual({ first: "2026-07-27", last: "2026-09-06" });
  });

  it("carries across years", () => {
    expect(gridRange({ year: 2027, month: 1 }, 1)).toEqual({ first: "2026-12-28", last: "2027-01-31" });
  });
});

describe("calendar date math", () => {
  it("moves months across years and keeps a day in a shorter month", () => {
    expect(shiftMonth({ year: 2026, month: 12 }, 1)).toEqual({ year: 2027, month: 1 });
    expect(shiftMonth({ year: 2026, month: 1 }, -1)).toEqual({ year: 2025, month: 12 });
    expect(shiftMonth({ year: 2026, month: 5 }, -17)).toEqual({ year: 2024, month: 12 });
    expect(addMonths("2026-01-31", 1)).toBe("2026-02-28");
    expect(addMonths("2024-03-31", -1)).toBe("2024-02-29");
    expect(addMonths("2026-12-15", 1)).toBe("2027-01-15");
    expect(daysInMonth({ year: 2024, month: 2 })).toBe(29);
    expect(monthOf("2026-10-17")).toEqual({ year: 2026, month: 10 });
  });

  it("moves days across weeks, months and years", () => {
    expect(addDays("2026-10-31", 1)).toBe("2026-11-01");
    expect(addDays("2026-01-03", -7)).toBe("2025-12-27");
    expect(addDays("2024-02-28", 1)).toBe("2024-02-29");
  });

  it("starts the week from the locale", () => {
    expect(weekStartOf("en-AU")).toBe(1);
    expect(weekStartOf("en-GB")).toBe(1);
    expect(weekStartOf("en-US")).toBe(0);
    expect(weekStartOf("not a locale!")).toBe(1);
    expect(weekdayNames("en-AU", 1).map((n) => n.short)).toEqual(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]);
    expect(weekdayNames("en-US", 0)[0]!.long).toBe("Sunday");
  });

  it("reads the day a daily notes location shows, and links to a day", () => {
    expect(dayOfLocation("/daily", "2026-10-17")).toBe("2026-10-17");
    expect(dayOfLocation("/daily/2026-10-05", "2026-10-17")).toBe("2026-10-05");
    expect(dayOfLocation("/daily/2026-10-05?x=1#top", "2026-10-17")).toBe("2026-10-05");
    expect(dayOfLocation("/notes/abc", "2026-10-17")).toBeUndefined();
    expect(dayOfLocation("/dailyish", "2026-10-17")).toBeUndefined();
    expect(dailyHref("2026-10-17", "2026-10-17")).toBe("/daily");
    expect(dailyHref("2026-10-05", "2026-10-17")).toBe("/daily/2026-10-05");
    expect(dailyQuery("2026-09-28", "2026-11-01")).toBe(
      'from dailynote where date >= "2026-09-28" && date <= "2026-11-01" select date',
    );
  });
});
