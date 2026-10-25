// Date math for the daily notes calendar in the right sidebar: which days a month's grid shows,
// the first day of the week for a locale, and moving between days and months. Days are
// `YYYY-MM-DD` strings in the browser's own time zone, as the planner uses.

import { plannerHref, plannerOfLocation } from "../planner/period";

/** A day of the week: 0 is Sunday, 1 Monday, up to 6 for Saturday, as `Date.getDay` counts. */
export type Weekday = 0 | 1 | 2 | 3 | 4 | 5 | 6;

/** A month: its year and its month, 1 to 12. */
export interface Month {
  year: number;
  month: number;
}

/** One cell of a month's grid. */
export interface GridDay {
  day: string;
  /** Its day of the month. */
  date: number;
  /** Whether it is in the month shown, not the end of the month before or the start of the next. */
  inMonth: boolean;
}

const pad = (n: number) => String(n).padStart(2, "0");

function iso(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** `y`-`m`-`d` as a day, any overflow carried as `Date` does (day 0 is the last of the month before). */
export function dayOf(y: number, m: number, d: number): string {
  return iso(new Date(y, m - 1, d));
}

function parts(day: string): [number, number, number] {
  const [y, m, d] = day.split("-").map(Number);
  return [y!, m!, d!];
}

/** Today, in the browser's own time zone. */
export function todayDay(): string {
  return iso(new Date());
}

/** The month `day` falls in. */
export function monthOf(day: string): Month {
  const [year, month] = parts(day);
  return { year, month };
}

/** The month `n` months after `m`. */
export function shiftMonth(m: Month, n: number): Month {
  const i = m.year * 12 + (m.month - 1) + n;
  return { year: Math.floor(i / 12), month: (((i % 12) + 12) % 12) + 1 };
}

export function sameMonth(a: Month, b: Month): boolean {
  return a.year === b.year && a.month === b.month;
}

export function daysInMonth(m: Month): number {
  return new Date(m.year, m.month, 0).getDate();
}

/** The day `n` days after `day`. */
export function addDays(day: string, n: number): string {
  const [y, m, d] = parts(day);
  return dayOf(y, m, d + n);
}

/** The same day of the month `n` months on, kept to the last day of a shorter month. */
export function addMonths(day: string, n: number): string {
  const [, , d] = parts(day);
  const to = shiftMonth(monthOf(day), n);
  return dayOf(to.year, to.month, Math.min(d, daysInMonth(to)));
}

/** The day of the week `day` falls on. */
export function weekdayOf(day: string): Weekday {
  const [y, m, d] = parts(day);
  return new Date(y, m - 1, d).getDay() as Weekday;
}

/**
 * The weeks of `m`'s grid, seven days each from `weekStart`: the first week starts on or before
 * the 1st and the last ends on or after the month's last day, so a grid has four to six weeks.
 */
export function monthGrid(m: Month, weekStart: Weekday): GridDay[][] {
  const lead = (new Date(m.year, m.month - 1, 1).getDay() - weekStart + 7) % 7;
  const total = Math.ceil((lead + daysInMonth(m)) / 7) * 7;
  const weeks: GridDay[][] = [];
  for (let i = 0; i < total; i++) {
    const date = new Date(m.year, m.month - 1, 1 - lead + i);
    if (i % 7 === 0) weeks.push([]);
    weeks[weeks.length - 1]!.push({ day: iso(date), date: date.getDate(), inMonth: date.getMonth() === m.month - 1 });
  }
  return weeks;
}

/** The first and last days `m`'s grid shows. */
export function gridRange(m: Month, weekStart: Weekday): { first: string; last: string } {
  const weeks = monthGrid(m, weekStart);
  return { first: weeks[0]![0]!.day, last: weeks[weeks.length - 1]![6]!.day };
}

/** Regions whose weeks start on Sunday, for browsers without `Intl.Locale` week info. */
const SUNDAY_REGIONS = new Set(["US", "CA", "MX", "BR", "JP", "KR", "TW", "HK", "IL", "PH", "IN", "ZA", "SA", "CO", "PE", "PR"]);

/**
 * The first day of the week in `locale`: Monday for `en-AU` and most of the world, Sunday for
 * `en-US`. The browser's week info decides where it has it.
 */
export function weekStartOf(locale: string): Weekday {
  try {
    const loc = new Intl.Locale(locale) as Intl.Locale & {
      getWeekInfo?: () => { firstDay: number };
      weekInfo?: { firstDay: number };
    };
    const info = typeof loc.getWeekInfo === "function" ? loc.getWeekInfo() : loc.weekInfo;
    if (info && Number.isInteger(info.firstDay)) return (info.firstDay % 7) as Weekday;
    const region = loc.maximize().region;
    return region && SUNDAY_REGIONS.has(region) ? 0 : 1;
  } catch {
    return 1;
  }
}

/** The seven weekday names from `weekStart` in `locale`, short ("Mon") and long ("Monday"). */
export function weekdayNames(locale: string, weekStart: Weekday): { short: string; long: string }[] {
  // 4 October 2026 is a Sunday.
  return Array.from({ length: 7 }, (_, i) => {
    const date = new Date(2026, 9, 4 + ((weekStart + i) % 7));
    return {
      short: date.toLocaleDateString(locale, { weekday: "short" }),
      long: date.toLocaleDateString(locale, { weekday: "long" }),
    };
  });
}

/** A month's title in `locale`, such as "October 2026". */
export function monthTitle(m: Month, locale: string): string {
  return new Date(m.year, m.month - 1, 1).toLocaleDateString(locale, { month: "long", year: "numeric" });
}

/** A day written out in `locale` for its cell's label, such as "Saturday 17 October 2026". */
export function dayLabel(day: string, locale: string): string {
  const [y, m, d] = parts(day);
  return new Date(y, m - 1, d).toLocaleDateString(locale, { weekday: "long", day: "numeric", month: "long", year: "numeric" });
}

/** The query for the daily notes of the days `first` to `last`. */
export function dailyQuery(first: string, last: string): string {
  return `from dailynote where date >= "${first}" && date <= "${last}" select date`;
}

/** Where `day`'s daily note is: today's is `/planner/day` itself, as the planner's Today goes. */
export function dailyHref(day: string, today: string): string {
  return plannerHref("day", day, today);
}

/** The day a planner location shows (the period's first day, `today` for the current day), or undefined for any other location. */
export function dayOfLocation(location: string, today: string): string | undefined {
  return plannerOfLocation(location, today)?.day;
}
