// What a `period_calendar` block shows (docs/vault-format.md, "period_calendar"): the notes of
// its query whose date falls in the note's period, each on every day of the period it covers.

import type { Block } from "../api";
import type { CalendarEvent } from "../views/narrow";
import { type Period, isPeriod, isoDay, shiftDay } from "./period";

/** The periods a calendar block can show: a day has no calendar of its own. */
export type CalendarPeriod = Exclude<Period, "day">;

export interface PeriodCalendarDef {
  period: CalendarPeriod;
  /** The notes shown. */
  query: string;
  /** The date, datetime or date range field of the notes that places them. */
  date: string;
}

export const DEFAULT_CALENDAR_QUERY = "from event";
export const DEFAULT_CALENDAR_DATE = "when";

/** A `period_calendar` block read with its defaults, or null when its period is not one it shows. */
export function readPeriodCalendar(block: Block): PeriodCalendarDef | null {
  const f = block.fields ?? {};
  const period = typeof f.period === "string" ? f.period : "";
  if (!isPeriod(period) || period === "day") return null;
  const query = typeof f.query === "string" && f.query.trim() ? f.query : DEFAULT_CALENDAR_QUERY;
  const date = typeof f.date === "string" && f.date.trim() ? f.date : DEFAULT_CALENDAR_DATE;
  return { period, query, date };
}

const DATE = /^\d{4}-\d{2}-\d{2}$/;

/**
 * The local days `e` covers, from `first` to `last` at most. An all-day event covers each day
 * from its start to the day before its (exclusive) end; a timed one each local day it is on,
 * a point in time just its own.
 */
export function eventDays(e: CalendarEvent, first: string, last: string): string[] {
  let from: string;
  let to: string;
  if (e.allDay || DATE.test(e.start)) {
    from = e.start.slice(0, 10);
    to = e.end ? shiftDay(e.end.slice(0, 10), -1) : from;
  } else {
    const start = new Date(e.start);
    if (Number.isNaN(start.getTime())) return [];
    from = isoDay(start);
    const end = e.end ? new Date(e.end) : null;
    // A span ending at midnight ends on the day before.
    to = end && end.getTime() > start.getTime() ? isoDay(new Date(end.getTime() - 1)) : from;
  }
  if (to < from) to = from;
  if (from < first) from = first;
  if (to > last) to = last;
  const out: string[] = [];
  for (let d = from; d <= to; d = shiftDay(d, 1)) out.push(d);
  return out;
}

/** The events of each day from `first` to `last`, by day, in the order given. */
export function eventsByDay(events: CalendarEvent[], first: string, last: string): Map<string, CalendarEvent[]> {
  const out = new Map<string, CalendarEvent[]>();
  for (const e of events) {
    for (const d of eventDays(e, first, last)) {
      const list = out.get(d);
      if (list) list.push(e);
      else out.set(d, [e]);
    }
  }
  return out;
}

/** An event's time of day, such as "09:30", or "" for an all-day one. */
export function eventTime(e: CalendarEvent, day: string): string {
  if (e.allDay || DATE.test(e.start)) return "";
  const start = new Date(e.start);
  // On a later day of an event that runs over midnight, it has been on since the day began.
  if (isoDay(start) !== day) return "";
  return `${String(start.getHours()).padStart(2, "0")}:${String(start.getMinutes()).padStart(2, "0")}`;
}

/** Order a day's events: all-day first, then by start. */
export function sortDay(events: CalendarEvent[]): CalendarEvent[] {
  return [...events].sort((a, b) => {
    const ad = a.allDay ? 0 : 1;
    const bd = b.allDay ? 0 : 1;
    if (ad !== bd) return ad - bd;
    const at = a.allDay ? a.start : new Date(a.start).toISOString();
    const bt = b.allDay ? b.start : new Date(b.start).toISOString();
    return at < bt ? -1 : at > bt ? 1 : a.title.localeCompare(b.title);
  });
}
