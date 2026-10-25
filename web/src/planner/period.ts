// Date math for the planner: the day, ISO week (Monday to Sunday), month, quarter and year a day
// falls in, stepping between them, their titles as the server writes them, and where each
// period's note is. Days are `YYYY-MM-DD` strings in the browser's own time zone.

/** A planner period. */
export type Period = "day" | "week" | "month" | "quarter" | "year";

export const PERIODS: Period[] = ["day", "week", "month", "quarter", "year"];

/** The switcher's label for each period. */
export const PERIOD_LABELS: Record<Period, string> = { day: "Day", week: "Week", month: "Month", quarter: "Quarter", year: "Year" };

/** The built-in note type of each period. */
export const PERIOD_TYPES: Record<Period, string> = {
  day: "dailynote",
  week: "weeklynote",
  month: "monthlynote",
  quarter: "quarterlynote",
  year: "yearlynote",
};

export function isPeriod(s: string | undefined): s is Period {
  return !!s && (PERIODS as string[]).includes(s);
}

/** The period whose built-in type is `type`, if it is one. */
export function periodOfType(type: string | undefined): Period | undefined {
  return PERIODS.find((p) => PERIOD_TYPES[p] === type);
}

export const DAY = /^\d{4}-\d{2}-\d{2}$/;

const pad = (n: number) => String(n).padStart(2, "0");

/** `d` as `YYYY-MM-DD` in the browser's own time zone. */
export function isoDay(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function parts(day: string): [number, number, number] {
  const [y, m, d] = day.split("-").map(Number);
  return [y!, m!, d!];
}

function dayOf(y: number, m: number, d: number): string {
  return isoDay(new Date(y, m - 1, d));
}

/** The day `days` after `day`. */
export function shiftDay(day: string, days: number): string {
  const [y, m, d] = parts(day);
  return dayOf(y, m, d + days);
}

/** Today, in the browser's own time zone. */
export function today(): string {
  return isoDay(new Date());
}

/** Monday 0 to Sunday 6. */
function isoWeekday(day: string): number {
  const [y, m, d] = parts(day);
  return (new Date(y, m - 1, d).getDay() + 6) % 7;
}

/** The ISO week `day` falls in: its week-year (the year of its Thursday) and its number. */
export function isoWeek(day: string): { year: number; week: number } {
  const thursday = shiftDay(day, 3 - isoWeekday(day));
  const [year, m, d] = parts(thursday);
  const jan1 = Date.UTC(year, 0, 1);
  const ordinal = Math.round((Date.UTC(year, m - 1, d) - jan1) / 86_400_000);
  return { year, week: Math.floor(ordinal / 7) + 1 };
}

/** The first day of the `period` `day` falls in. */
export function periodStart(period: Period, day: string): string {
  const [y, m] = parts(day);
  switch (period) {
    case "day":
      return day;
    case "week":
      return shiftDay(day, -isoWeekday(day));
    case "month":
      return dayOf(y, m, 1);
    case "quarter":
      return dayOf(y, m - ((m - 1) % 3), 1);
    case "year":
      return dayOf(y, 1, 1);
  }
}

/** The start of the period `n` periods after the one `day` falls in. */
export function shiftPeriod(period: Period, day: string, n: number): string {
  const start = periodStart(period, day);
  const [y, m] = parts(start);
  switch (period) {
    case "day":
      return shiftDay(start, n);
    case "week":
      return shiftDay(start, 7 * n);
    case "month":
      return dayOf(y, m + n, 1);
    case "quarter":
      return dayOf(y, m + 3 * n, 1);
    case "year":
      return dayOf(y + n, 1, 1);
  }
}

/** The last day of the period `day` falls in. */
export function periodEnd(period: Period, day: string): string {
  return shiftDay(shiftPeriod(period, day, 1), -1);
}

/** Every day of the period `day` falls in, in order. */
export function periodDays(period: Period, day: string): string[] {
  const out: string[] = [];
  const end = periodEnd(period, day);
  for (let d = periodStart(period, day); d <= end; d = shiftDay(d, 1)) out.push(d);
  return out;
}

const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
export const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

/** `day` written out the way the server titles a daily note, such as `Tuesday 13 October 2026`. */
export function longDay(day: string): string {
  const [y, m, d] = parts(day);
  return `${WEEKDAYS[new Date(y, m - 1, d).getDay()]} ${d} ${MONTHS[m - 1]} ${y}`;
}

/**
 * The title the server gives the note of the period `day` falls in: `Tuesday 13 October 2026`,
 * `Week 43, 2026` (the ISO week-year), `October 2026`, `Q4 2026` or `2026`.
 */
export function periodTitle(period: Period, day: string): string {
  const [y, m] = parts(day);
  switch (period) {
    case "day":
      return longDay(day);
    case "week": {
      const w = isoWeek(day);
      return `Week ${w.week}, ${w.year}`;
    }
    case "month":
      return `${MONTHS[m - 1]} ${y}`;
    case "quarter":
      return `Q${Math.floor((m - 1) / 3) + 1} ${y}`;
    case "year":
      return String(y);
  }
}

/** Where the note of the period `day` falls in is: the current period's is `/planner/<period>`. */
export function plannerHref(period: Period, day: string, now: string): string {
  const start = periodStart(period, day);
  return start === periodStart(period, now) ? `/planner/${period}` : `/planner/${period}/${start}`;
}

/** The period and day a planner location shows (`now` for the current period), or undefined for any other location. */
export function plannerOfLocation(location: string, now: string): { period: Period; day: string } | undefined {
  const path = location.split(/[?#]/)[0]!.replace(/\/$/, "");
  const daily = /^\/daily(?:\/(\d{4}-\d{2}-\d{2}))?$/.exec(path);
  if (daily) return { period: "day", day: daily[1] ?? now };
  const m = /^\/planner(?:\/([a-z]+)(?:\/(\d{4}-\d{2}-\d{2}))?)?$/.exec(path);
  if (!m) return undefined;
  const period = m[1] === undefined ? lastPeriod() : m[1];
  if (!isPeriod(period)) return undefined;
  return { period, day: periodStart(period, m[2] ?? now) };
}

export const PERIOD_KEY = "thevault.plannerPeriod";

/** The period last chosen in this browser, `day` at first. */
export function lastPeriod(): Period {
  try {
    const v = localStorage.getItem(PERIOD_KEY);
    return isPeriod(v ?? undefined) ? (v as Period) : "day";
  } catch {
    return "day";
  }
}

/** Remember `period` as the last chosen, per browser. */
export function rememberPeriod(period: Period) {
  try {
    localStorage.setItem(PERIOD_KEY, period);
  } catch {
    // Private browsing: the choice lasts until the page closes.
  }
}
