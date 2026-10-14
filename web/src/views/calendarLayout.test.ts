import { describe, expect, it } from "vitest";
import { COMPACT_CALENDAR, calendarLayout } from "./DateViews";

describe("calendarLayout", () => {
  it("keeps everything on one toolbar when there is room", () => {
    const wide = calendarLayout(COMPACT_CALENDAR);
    expect(wide.compact).toBe(false);
    expect(wide.headerToolbar.right).toBe("dayGridMonth,timeGridWeek,timeGridDay");
    expect(wide.footerToolbar).toBe(false);
    expect(wide.dayHeaderFormat.weekday).toBe("short");
  });

  it("moves the view buttons to a row underneath and shortens weekdays when narrow", () => {
    const narrow = calendarLayout(COMPACT_CALENDAR - 1);
    expect(narrow.compact).toBe(true);
    expect(narrow.headerToolbar).toEqual({ left: "prev,next", center: "title", right: "today" });
    expect(narrow.footerToolbar).toEqual({ center: "dayGridMonth,timeGridWeek,timeGridDay" });
    expect(narrow.dayHeaderFormat.weekday).toBe("narrow");
  });
});
