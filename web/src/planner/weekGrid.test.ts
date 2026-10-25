import { describe, expect, it } from "vitest";
import type { CalendarEvent } from "../views/narrow";
import { MIN_EVENT_MINUTES, allDayBars, assignLanes, hourRange, scrollHour, timedSegments, type TimedSegment } from "./weekGrid";

// The week time grid's layout: hours shown, where timed events sit, how overlapping ones share
// a column, and how all-day events span the row above. The week is Monday 19 to Sunday 25
// October 2026, in the browser's own time zone.

const MON = "2026-10-19";
const SUN = "2026-10-25";

/** A timed event on October `day` from `from` to `to`, as [hour, minute], local time. */
function timed(id: string, day: number, from: [number, number], to?: [number, number], toDay = day): CalendarEvent {
  const start = new Date(2026, 9, day, from[0], from[1]).toISOString();
  const end = to ? new Date(2026, 9, toDay, to[0], to[1]).toISOString() : undefined;
  return { id, title: id, start, ...(end ? { end } : {}), allDay: false };
}

const allDay = (id: string, start: string, end?: string): CalendarEvent => ({ id, title: id, start, ...(end ? { end } : {}), allDay: true });

const seg = (id: string, start: number, end: number): TimedSegment => ({
  event: { id, title: id, start: "", allDay: false },
  day: MON,
  start,
  end,
  continuesBefore: false,
  continuesAfter: false,
  lane: -1,
  lanes: -1,
});

describe("timed events", () => {
  it("places an event by its start and sizes it by its length", () => {
    const [s] = timedSegments([timed("a", 20, [9, 30], [11, 0])], MON, SUN).get("2026-10-20")!;
    expect(s).toMatchObject({ start: 570, end: 660, lane: 0, lanes: 1, continuesBefore: false, continuesAfter: false });
  });

  it("draws a short event or a point in time at least the minimum length", () => {
    const m = timedSegments([timed("short", 21, [9, 0], [9, 5]), timed("point", 22, [14, 0])], MON, SUN);
    expect(m.get("2026-10-21")![0]).toMatchObject({ start: 540, end: 540 + MIN_EVENT_MINUTES });
    expect(m.get("2026-10-22")![0]).toMatchObject({ start: 840, end: 840 + MIN_EVENT_MINUTES });
  });

  it("clips an event over midnight to each day it is on", () => {
    const m = timedSegments([timed("late", 20, [22, 0], [2, 0], 21)], MON, SUN);
    expect(m.get("2026-10-20")![0]).toMatchObject({ start: 1320, end: 1440, continuesBefore: false, continuesAfter: true });
    expect(m.get("2026-10-21")![0]).toMatchObject({ start: 0, end: 120, continuesBefore: true, continuesAfter: false });
  });

  it("clips an event over the week's edges to the days inside it", () => {
    const m = timedSegments([timed("before", 18, [20, 0], [1, 0], 19), timed("after", 25, [23, 0], [3, 0], 26)], MON, SUN);
    expect([...m.keys()].sort()).toEqual([MON, SUN]);
    expect(m.get(MON)![0]).toMatchObject({ start: 0, end: 60, continuesBefore: true });
    expect(m.get(SUN)![0]).toMatchObject({ start: 1380, end: 1440, continuesAfter: true });
  });

  it("leaves all-day and date-only events to the all-day row", () => {
    const m = timedSegments([allDay("a", "2026-10-20"), { id: "b", title: "b", start: "2026-10-21", allDay: false }], MON, SUN);
    expect(m.size).toBe(0);
  });
});

describe("lanes", () => {
  it("puts overlapping events side by side, sharing the width", () => {
    const segs = [seg("a", 540, 600), seg("b", 570, 660), seg("c", 720, 780)];
    assignLanes(segs);
    expect(segs.map((s) => [s.event.id, s.lane, s.lanes])).toEqual([
      ["a", 0, 2],
      ["b", 1, 2],
      ["c", 0, 1],
    ]);
  });

  it("reuses a lane that has come free within a group", () => {
    // a runs long; b ends before d starts, so d takes b's lane and the group stays two wide.
    const segs = [seg("a", 540, 720), seg("b", 560, 600), seg("d", 610, 650)];
    assignLanes(segs);
    expect(segs.map((s) => [s.event.id, s.lane, s.lanes])).toEqual([
      ["a", 0, 2],
      ["b", 1, 2],
      ["d", 1, 2],
    ]);
  });

  it("gives three at once three lanes, and an event that only touches another none of its", () => {
    const segs = [seg("a", 600, 660), seg("b", 600, 630), seg("c", 615, 645), seg("e", 660, 690)];
    assignLanes(segs);
    expect(segs.map((s) => [s.event.id, s.lane, s.lanes])).toEqual([
      ["a", 0, 3],
      ["b", 1, 3],
      ["c", 2, 3],
      ["e", 0, 1],
    ]);
  });

  it("lays out each day's events apart", () => {
    const m = timedSegments([timed("x", 20, [9, 0], [10, 0]), timed("y", 20, [9, 30], [10, 30]), timed("z", 21, [9, 0], [10, 0])], MON, SUN);
    expect(m.get("2026-10-20")!.map((s) => s.lanes)).toEqual([2, 2]);
    expect(m.get("2026-10-21")!.map((s) => s.lanes)).toEqual([1]);
  });
});

describe("the hours shown", () => {
  it("shows 07:00 to 21:00 with nothing outside them", () => {
    expect(hourRange([])).toEqual({ first: 7, last: 21 });
    expect(hourRange([seg("a", 540, 600)])).toEqual({ first: 7, last: 21 });
  });

  it("widens to whole hours to take in an early or late event", () => {
    expect(hourRange([seg("early", 6 * 60 + 15, 7 * 60)])).toEqual({ first: 6, last: 21 });
    expect(hourRange([seg("late", 21 * 60, 22 * 60 + 10)])).toEqual({ first: 7, last: 23 });
    expect(hourRange([seg("overnight", 0, 120)])).toEqual({ first: 0, last: 21 });
  });

  it("opens at the earliest event's hour, or 08:00", () => {
    expect(scrollHour([])).toBe(8);
    expect(scrollHour([seg("a", 600, 660), seg("b", 6 * 60 + 30, 7 * 60)])).toBe(6);
    expect(scrollHour([{ ...seg("overnight", 0, 120), continuesBefore: true }, seg("a", 600, 660)])).toBe(10);
  });
});

describe("the all-day row", () => {
  it("spans each event over the days it covers, from its first column", () => {
    const [bar] = allDayBars([allDay("offsite", "2026-10-21", "2026-10-24")], MON);
    expect(bar).toMatchObject({ column: 2, span: 3, row: 0, days: ["2026-10-21", "2026-10-22", "2026-10-23"] });
    const [one] = allDayBars([allDay("one", "2026-10-25")], MON);
    expect(one).toMatchObject({ column: 6, span: 1, continuesBefore: false, continuesAfter: false });
  });

  it("clips an event over the week's edges", () => {
    const bars = allDayBars([allDay("conf", "2026-10-17", "2026-10-21"), allDay("trip", "2026-10-24", "2026-10-30")], MON);
    expect(bars.map((b) => [b.event.id, b.column, b.span, b.continuesBefore, b.continuesAfter])).toEqual([
      ["conf", 0, 2, true, false],
      ["trip", 5, 2, false, true],
    ]);
  });

  it("stacks overlapping events in rows and reuses a row once it is free", () => {
    const bars = allDayBars(
      [allDay("a", "2026-10-19", "2026-10-22"), allDay("b", "2026-10-20", "2026-10-21"), allDay("c", "2026-10-22", "2026-10-23"), allDay("d", "2026-10-21")],
      MON,
    );
    expect(Object.fromEntries(bars.map((b) => [b.event.id, b.row]))).toEqual({ a: 0, b: 1, d: 1, c: 0 });
  });

  it("leaves out timed events and those outside the week", () => {
    expect(allDayBars([timed("t", 20, [9, 0], [10, 0]), allDay("x", "2026-11-02")], MON)).toEqual([]);
  });
});
