// The layout of a week's time grid: which hours it shows, where each timed event sits in its
// day's column and how overlapping ones share it, and how all-day events span the row above.
// Times are minutes from the local midnight of their day; days are `YYYY-MM-DD` strings.

import type { CalendarEvent } from "../views/narrow";
import { eventDays } from "./periodCalendar";
import { shiftDay } from "./period";

/** The hours a week shows with nothing outside them: 07:00 to 21:00. */
export const DEFAULT_FIRST_HOUR = 7;
export const DEFAULT_LAST_HOUR = 21;

/** The shortest an event is drawn, so a point in time or a five-minute call stays readable. */
export const MIN_EVENT_MINUTES = 30;

/** Where the grid opens when the week has no timed events. */
export const DEFAULT_SCROLL_HOUR = 8;

const DAY_MINUTES = 24 * 60;
const DATE = /^\d{4}-\d{2}-\d{2}$/;

/** Whether `e` goes in the all-day row rather than the time grid. */
export function isAllDay(e: CalendarEvent): boolean {
  return e.allDay || DATE.test(e.start);
}

function midnight(day: string): number {
  const [y, m, d] = day.split("-").map(Number);
  return new Date(y!, m! - 1, d!).getTime();
}

/** A timed event's piece on one day, placed in a lane of the day's column. */
export interface TimedSegment {
  event: CalendarEvent;
  day: string;
  /** Minutes from midnight, clipped to the day. */
  start: number;
  /** Minutes from midnight, clipped to the day, at least MIN_EVENT_MINUTES after start. */
  end: number;
  /** The event began on an earlier day, or runs on to a later one. */
  continuesBefore: boolean;
  continuesAfter: boolean;
  lane: number;
  /** The lanes of the group of overlapping events it is in. */
  lanes: number;
}

/**
 * The timed events of each day from `first` to `last`, each clipped to its day and laid out in
 * lanes: events that overlap sit side by side, each `1 / lanes` of the column wide.
 */
export function timedSegments(events: CalendarEvent[], first: string, last: string): Map<string, TimedSegment[]> {
  const out = new Map<string, TimedSegment[]>();
  for (const e of events) {
    if (isAllDay(e)) continue;
    const start = new Date(e.start).getTime();
    if (Number.isNaN(start)) continue;
    const endAt = e.end ? new Date(e.end).getTime() : Number.NaN;
    const end = Number.isNaN(endAt) || endAt < start ? start : endAt;
    for (const day of eventDays(e, first, last)) {
      const zero = midnight(day);
      const from = Math.max(0, (start - zero) / 60_000);
      const to = Math.min(DAY_MINUTES, (end - zero) / 60_000);
      const seg: TimedSegment = {
        event: e,
        day,
        start: from,
        end: Math.min(DAY_MINUTES, Math.max(to, from + MIN_EVENT_MINUTES)),
        continuesBefore: start < zero,
        continuesAfter: end > zero + DAY_MINUTES * 60_000,
        lane: 0,
        lanes: 1,
      };
      const list = out.get(day);
      if (list) list.push(seg);
      else out.set(day, [seg]);
    }
  }
  for (const list of out.values()) assignLanes(list);
  return out;
}

/**
 * Sort a day's segments by start and give each a lane: the first lane free when it starts,
 * within its group of transitively overlapping segments, all of which share the group's width.
 */
export function assignLanes(segs: TimedSegment[]): void {
  segs.sort((a, b) => a.start - b.start || b.end - a.end || a.event.title.localeCompare(b.event.title));
  let group: TimedSegment[] = [];
  let laneEnds: number[] = [];
  let groupEnd = -1;
  const close = () => {
    for (const s of group) s.lanes = laneEnds.length;
    group = [];
    laneEnds = [];
  };
  for (const s of segs) {
    if (s.start >= groupEnd) close();
    let lane = laneEnds.findIndex((end) => end <= s.start);
    if (lane < 0) {
      lane = laneEnds.length;
      laneEnds.push(s.end);
    } else laneEnds[lane] = s.end;
    s.lane = lane;
    group.push(s);
    groupEnd = Math.max(groupEnd, s.end);
  }
  close();
}

/**
 * The whole hours the grid shows: 07:00 to 21:00, widened to take in every timed segment.
 * `last` is the hour the grid ends at, so the grid has `last - first` rows.
 */
export function hourRange(segments: Iterable<TimedSegment>): { first: number; last: number } {
  let first = DEFAULT_FIRST_HOUR;
  let last = DEFAULT_LAST_HOUR;
  for (const s of segments) {
    first = Math.min(first, Math.floor(s.start / 60));
    last = Math.max(last, Math.ceil(s.end / 60));
  }
  return { first, last };
}

/**
 * The hour the grid opens scrolled to: that of the week's earliest timed event to start, or
 * 08:00. The rest of an event from the night before starts at midnight but does not count.
 */
export function scrollHour(segments: Iterable<TimedSegment>): number {
  let min = Infinity;
  for (const s of segments) if (!s.continuesBefore) min = Math.min(min, s.start);
  return Number.isFinite(min) ? Math.floor(min / 60) : DEFAULT_SCROLL_HOUR;
}

/** An all-day event's bar in the row above the grid. */
export interface AllDayBar {
  event: CalendarEvent;
  /** The days of the week it covers. */
  days: string[];
  /** Its first day, Monday 0 to Sunday 6, and how many days it spans. */
  column: number;
  span: number;
  /** Its row, from 0, so bars that overlap stack. */
  row: number;
  continuesBefore: boolean;
  continuesAfter: boolean;
}

/** The all-day and date-only events of the seven days from `first`, as bars stacked in rows. */
export function allDayBars(events: CalendarEvent[], first: string): AllDayBar[] {
  const last = shiftDay(first, 6);
  const week = Array.from({ length: 7 }, (_, i) => shiftDay(first, i));
  const bars: AllDayBar[] = [];
  for (const e of events) {
    if (!isAllDay(e)) continue;
    const days = eventDays(e, first, last);
    if (!days.length) continue;
    // Its own last day, before the exclusive end.
    const lastDay = e.end ? shiftDay(e.end.slice(0, 10), -1) : e.start.slice(0, 10);
    bars.push({
      event: e,
      days,
      column: week.indexOf(days[0]!),
      span: days.length,
      row: 0,
      continuesBefore: e.start.slice(0, 10) < first,
      continuesAfter: lastDay > last,
    });
  }
  bars.sort((a, b) => a.column - b.column || b.span - a.span || a.event.title.localeCompare(b.event.title));
  const rowEnds: number[] = [];
  for (const b of bars) {
    let row = rowEnds.findIndex((end) => end <= b.column);
    if (row < 0) row = rowEnds.length;
    rowEnds[row] = b.column + b.span;
    b.row = row;
  }
  return bars;
}

/** Minutes as "HH:MM", 24-hour. */
export function clock(minutes: number): string {
  const m = Math.round(minutes);
  return `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}
