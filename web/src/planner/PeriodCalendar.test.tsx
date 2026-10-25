import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { Block, Row, TypeDef } from "../api";

// The `period_calendar` block of a planner note: the events whose time overlaps the note's
// period, each on every day of the period it covers, every day a link to its daily note.

const query = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, query } };
});

const eventType: TypeDef = {
  name: "event",
  label: "Event",
  plural: "Events",
  description: null,
  icon: "calendar",
  colour: "blue",
  fields: [
    { name: "when", label: "When", description: null, kind: "date_range", of: "date_or_datetime", required: true, computed: false, multiple: false, compound: false },
  ],
  templates: [],
} as TypeDef;

vi.mock("../session", async (orig) => {
  const real = await orig<typeof import("../session")>();
  return { ...real, types: () => [eventType] };
});

const { PeriodCalendar } = await import("./PeriodCalendar");
const { eventDays, eventsByDay, readPeriodCalendar } = await import("./periodCalendar");

const row = (id: string, title: string, start: string, end: string): Row => ({ id, type: "event", title, fields: { when: { start, end } } });

/** Spans into the week from the Sunday before. */
const SPANNING = row("01A", "Conference", "2026-10-17", "2026-10-20");
/** All day, Wednesday to Friday. */
const ALL_DAY = row("01B", "Offsite", "2026-10-21", "2026-10-23");
/** One morning, local time. */
const MEETING = row("01C", "Standup", new Date(2026, 9, 24, 9, 30).toISOString(), new Date(2026, 9, 24, 10, 0).toISOString());

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

// Today is far from the periods shown, so every day link names its date.
beforeAll(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2031, 5, 15, 12, 0));
});
afterAll(() => vi.useRealTimers());

const block = (period: string, extra: Record<string, string> = {}): Block => ({ kind: "period_calendar", fields: { period, ...extra } });

describe("reading the block", () => {
  it("takes its period and defaults to events placed by when", () => {
    expect(readPeriodCalendar(block("week"))).toEqual({ period: "week", query: "from event", date: "when" });
    expect(readPeriodCalendar(block("year", { query: "from task", date: "due" }))).toEqual({ period: "year", query: "from task", date: "due" });
    expect(readPeriodCalendar(block("day"))).toBeNull();
    expect(readPeriodCalendar(block("decade"))).toBeNull();
  });
});

describe("the days an event covers", () => {
  it("clips a span to the period", () => {
    const e = { id: "a", title: "a", start: "2026-10-17", end: "2026-10-21", allDay: true };
    expect(eventDays(e, "2026-10-19", "2026-10-25")).toEqual(["2026-10-19", "2026-10-20"]);
  });

  it("puts a timed event on each local day it is on", () => {
    const start = new Date(2026, 9, 20, 22, 0).toISOString();
    const end = new Date(2026, 9, 21, 2, 0).toISOString();
    expect(eventDays({ id: "a", title: "a", start, end, allDay: false }, "2026-10-19", "2026-10-25")).toEqual(["2026-10-20", "2026-10-21"]);
    const midnight = new Date(2026, 9, 21, 0, 0).toISOString();
    expect(eventDays({ id: "a", title: "a", start, end: midnight, allDay: false }, "2026-10-19", "2026-10-25")).toEqual(["2026-10-20"]);
  });

  it("leaves out an event outside the period", () => {
    const e = { id: "a", title: "a", start: "2026-11-02", allDay: true };
    expect(eventsByDay([e], "2026-10-19", "2026-10-25").size).toBe(0);
  });
});

describe("the week calendar", () => {
  it("shows Monday to Sunday with each event on the days it covers", async () => {
    query.mockResolvedValue({ rows: [SPANNING, ALL_DAY, MEETING], next: null, total: 3 });
    render(() => <PeriodCalendar block={block("week")} date="2026-10-21" />);
    const days = await screen.findAllByTestId("period-day");
    expect(days.map((a) => a.getAttribute("href"))).toEqual([
      "/planner/day/2026-10-19",
      "/planner/day/2026-10-20",
      "/planner/day/2026-10-21",
      "/planner/day/2026-10-22",
      "/planner/day/2026-10-23",
      "/planner/day/2026-10-24",
      "/planner/day/2026-10-25",
    ]);
    expect(days[0]!.getAttribute("aria-label")).toBe("Monday 19 October 2026");
    // The query is narrowed to the week, with `this` the note.
    const q = String(query.mock.calls[0]![0]);
    expect(q.startsWith("from event where")).toBe(true);
    expect(q).toContain('when.start < "2026-10-26"');
    expect(q).toContain('when.end >= "2026-10-19"');

    // A timed event sits in its day's column; an all-day one spans the days it covers above.
    const on = (day: string) =>
      [...document.querySelectorAll(`.week-col[data-day="${day}"] [data-testid="period-event"], [data-testid="period-event"][data-days~="${day}"]`)].map(
        (e) => e.querySelector(".period-event-title")!.textContent,
      );
    await waitFor(() => expect(on("2026-10-19")).toEqual(["Conference"]));
    expect(on("2026-10-20")).toEqual(["Conference"]);
    expect(on("2026-10-21")).toEqual(["Offsite"]);
    expect(on("2026-10-22")).toEqual(["Offsite"]);
    expect(on("2026-10-23")).toEqual(["Offsite"]);
    expect(on("2026-10-24")).toEqual(["Standup"]);
    expect(on("2026-10-25")).toEqual([]);
    const standup = document.querySelector('.week-col[data-day="2026-10-24"] [data-testid="period-event"]')!;
    expect(standup.getAttribute("href")).toBe("/notes/01C");
    expect(standup.querySelector(".period-event-time")?.textContent).toBe("09:30");
    expect(standup.getAttribute("title")).toBe("09:30 Standup");
    // Its block starts 2.5 hour rows below 07:00 and is half a row tall.
    expect((standup as HTMLElement).style.top).toBe("calc(var(--hour-height) * 2.5)");
    expect((standup as HTMLElement).style.height).toBe("calc(var(--hour-height) * 0.5 - 2px)");
    // The all-day events span their days of the week in rows of their own.
    const bar = (title: string) => screen.getByTitle(title) as HTMLElement;
    expect(bar("Conference").style.gridColumn).toBe("2 / span 2");
    expect(bar("Offsite").style.gridColumn).toBe("4 / span 3");
    expect(bar("Conference").classList.contains("continues-before")).toBe(true);
    // The hours down the side run from 07:00 to 20:00, the start of the last row.
    const hours = [...document.querySelectorAll(".week-hour")].map((h) => h.textContent);
    expect(hours[0]).toBe("07:00");
    expect(hours.at(-1)).toBe("20:00");
  });

  it("lists the days one under another in a narrow pane", async () => {
    const width = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(400);
    try {
      query.mockResolvedValue({ rows: [ALL_DAY, MEETING], next: null, total: 2 });
      render(() => <PeriodCalendar block={block("week")} date="2026-10-21" />);
      expect(await screen.findByTestId("period-week-list")).toBeTruthy();
      expect(screen.queryByTestId("period-week-grid")).toBeNull();
      expect(screen.getAllByTestId("period-day")).toHaveLength(7);
      await waitFor(() => expect(document.querySelectorAll('[data-day="2026-10-22"] [data-testid="period-event"]')).toHaveLength(1));
    } finally {
      width.mockRestore();
    }
  });

  it("shows the problem in place of the calendar for a field it cannot place notes by", () => {
    render(() => <PeriodCalendar block={block("week", { date: "nope" })} date="2026-10-21" />);
    expect(screen.getByTestId("view-problem").textContent).toMatch(/No type in this query has a field named nope/);
    expect(screen.queryAllByTestId("period-day")).toHaveLength(0);
    expect(query).not.toHaveBeenCalled();
  });

  it("shows a failed query in place of the calendar", async () => {
    query.mockRejectedValue(new Error("the query does not parse"));
    render(() => <PeriodCalendar block={block("week", { query: "from event where (" })} date="2026-10-21" />);
    expect((await screen.findByRole("alert")).textContent).toBe("the query does not parse");
    expect(screen.queryAllByTestId("period-day")).toHaveLength(0);
  });

  it("asks the note to have a date first", () => {
    render(() => <PeriodCalendar block={block("week")} date="" />);
    expect(screen.getByText(/once it has a date/)).toBeTruthy();
    expect(query).not.toHaveBeenCalled();
  });
});

describe("the month, quarter and year calendars", () => {
  it("shows a month from Monday with only its own days", async () => {
    query.mockResolvedValue({ rows: [SPANNING], next: null, total: 1 });
    render(() => <PeriodCalendar block={block("month")} date="2026-10-15" />);
    const days = await screen.findAllByTestId("period-day");
    expect(days).toHaveLength(31);
    expect(days[0]!.getAttribute("href")).toBe("/planner/day/2026-10-01");
    await waitFor(() => expect(document.querySelectorAll('[data-testid="period-event"]')).toHaveLength(4));
  });

  it("shows a quarter as its three months", async () => {
    query.mockResolvedValue({ rows: [], next: null, total: 0 });
    render(() => <PeriodCalendar block={block("quarter")} date="2026-11-20" />);
    await screen.findAllByTestId("period-day");
    expect(screen.getAllByRole("table")).toHaveLength(3);
    expect(screen.getByRole("link", { name: "October 2026" }).getAttribute("href")).toBe("/planner/month/2026-10-01");
    expect(screen.getByRole("link", { name: "December 2026" })).toBeTruthy();
    expect(String(query.mock.calls[0]![0])).toContain('when.start < "2027-01-01"');
  });

  it("counts a year's days and lists a clicked day's events", async () => {
    query.mockResolvedValue({ rows: [SPANNING, ALL_DAY, MEETING], next: null, total: 3 });
    render(() => <PeriodCalendar block={block("year")} date="2026-01-01" />);
    expect(await screen.findAllByRole("table")).toHaveLength(12);
    const day = (d: string) => document.querySelector<HTMLButtonElement>(`[data-testid="period-year-day"][data-day="${d}"]`)!;
    await waitFor(() => expect(day("2026-10-21").querySelector(".period-count")?.textContent).toBe("1"));
    expect(day("2026-10-17").querySelector(".period-count")?.textContent).toBe("1");
    expect(day("2026-10-25").querySelector(".period-count")).toBeNull();
    fireEvent.click(day("2026-10-24"));
    const picked = screen.getByTestId("period-picked");
    expect(picked.querySelector('[data-testid="period-day"]')!.getAttribute("href")).toBe("/planner/day/2026-10-24");
    expect(picked.querySelector('[data-testid="period-event"]')!.getAttribute("href")).toBe("/notes/01C");
  });
});
