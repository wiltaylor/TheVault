import { describe, expect, it } from "vitest";
import { isoDay, longDay, shiftDay } from "./DailyNotes";

describe("daily note days", () => {
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
