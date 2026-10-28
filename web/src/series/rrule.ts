// An event series' `rule` (event-series E6): its RFC 5545 lines, `RRULE:...` and optionally
// `EXDATE`/`RDATE` lines as Google returns them. The rule builder edits one plain `RRULE` line:
// a frequency, an interval, weekdays for a weekly rule, a day of the month or "the 2nd Tuesday"
// for a monthly one, and an end. Anything else, an `EXDATE` or `RDATE` line, a second `RRULE`, or
// a part such as `BYSETPOS`, is shown and edited as the raw lines. Every rule, raw or not, reads
// as a summary such as "Every 2 weeks on Mon and Wed, until 30 June 2027".

import type { FieldDef } from "../api";
import { typeDef } from "../session";

/** The type a series note is, and its rule field. */
export const SERIES_TYPE = "event_series";
export const RULE_FIELD = "rule";

export type Freq = "daily" | "weekly" | "monthly" | "yearly";
export const FREQS: Freq[] = ["daily", "weekly", "monthly", "yearly"];

/** RFC 5545 weekdays, Monday first. */
export const WEEKDAYS = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"] as const;
export type Weekday = (typeof WEEKDAYS)[number];
export const WEEKDAY_SHORT: Record<Weekday, string> = { MO: "Mon", TU: "Tue", WE: "Wed", TH: "Thu", FR: "Fri", SA: "Sat", SU: "Sun" };
export const WEEKDAY_LONG: Record<Weekday, string> = {
  MO: "Monday",
  TU: "Tuesday",
  WE: "Wednesday",
  TH: "Thursday",
  FR: "Friday",
  SA: "Saturday",
  SU: "Sunday",
};

/** Which day of the month a monthly rule falls on. */
export type MonthlyOn =
  /** No `BY` part: the day of the month the first occurrence is on. */
  | { by: "start" }
  /** `BYMONTHDAY=n`; -1 is the last day. */
  | { by: "day"; day: number }
  /** `BYDAY=2TU`: the nth weekday of the month; -1 is the last. */
  | { by: "weekday"; nth: number; weekday: Weekday };

export type RuleEnd = { kind: "never" } | { kind: "until"; date: string } | { kind: "count"; count: number };

/** What the builder edits: one `RRULE` line. */
export interface RuleModel {
  freq: Freq;
  interval: number;
  /** A weekly rule's days; none is the first occurrence's weekday. */
  weekdays: Weekday[];
  monthly: MonthlyOn;
  end: RuleEnd;
}

export function defaultRule(): RuleModel {
  return { freq: "weekly", interval: 1, weekdays: [], monthly: { by: "start" }, end: { kind: "never" } };
}

const FREQ_OF: Record<string, Freq> = { DAILY: "daily", WEEKLY: "weekly", MONTHLY: "monthly", YEARLY: "yearly" };

/** The rule lines, trimmed, without empty ones. */
export function ruleLines(value: unknown): string[] {
  const items = Array.isArray(value) ? value : typeof value === "string" ? value.split("\n") : [];
  return items.filter((l): l is string => typeof l === "string").map((l) => l.trim()).filter((l) => l !== "");
}

/** The property name of a line: `RRULE`, `EXDATE`, `RDATE`, ... upper-cased. */
function property(line: string): string {
  return (line.split(/[:;]/, 1)[0] ?? "").toUpperCase();
}

/** The parts of an `RRULE:` line's value, names upper-cased, in order. */
export function ruleParts(line: string): [string, string][] | null {
  const at = line.indexOf(":");
  if (at < 0) return null;
  const out: [string, string][] = [];
  for (const p of line.slice(at + 1).split(";")) {
    if (!p.trim()) continue;
    const eq = p.indexOf("=");
    if (eq <= 0) return null;
    out.push([p.slice(0, eq).trim().toUpperCase(), p.slice(eq + 1).trim().toUpperCase()]);
  }
  return out;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** `YYYY-MM-DD` from an `UNTIL` value: a date as it is, a UTC time as the local day it falls on. */
function untilDate(v: string): string | null {
  const m = /^(\d{4})(\d{2})(\d{2})(?:T(\d{2})(\d{2})(\d{2})(Z?))?$/.exec(v);
  if (!m) return null;
  if (m[4] !== undefined && m[7] === "Z") {
    const t = new Date(Date.UTC(+m[1]!, +m[2]! - 1, +m[3]!, +m[4]!, +m[5]!, +m[6]!));
    return `${t.getFullYear()}-${pad(t.getMonth() + 1)}-${pad(t.getDate())}`;
  }
  return `${m[1]}-${m[2]}-${m[3]}`;
}

/**
 * An `UNTIL` value for the end of local day `date`: the date itself for an all-day series, as
 * RFC 5545 asks when the start is a date; else the last second of that day here, in UTC.
 */
function untilValue(date: string, allDay: boolean): string {
  const [y, m, d] = date.split("-").map(Number) as [number, number, number];
  if (allDay) return `${y}${pad(m)}${pad(d)}`;
  const t = new Date(y, m - 1, d, 23, 59, 59);
  return `${t.getUTCFullYear()}${pad(t.getUTCMonth() + 1)}${pad(t.getUTCDate())}T${pad(t.getUTCHours())}${pad(t.getUTCMinutes())}${pad(t.getUTCSeconds())}Z`;
}

const isWeekday = (s: string): s is Weekday => (WEEKDAYS as readonly string[]).includes(s);

/** The builder's model of one `RRULE` line, or why the builder cannot show it. */
export function modelOf(line: string): RuleModel | string {
  const parts = ruleParts(line);
  if (!parts) return "The RRULE line does not read as NAME=VALUE parts.";
  const m = defaultRule();
  const seen = new Set<string>();
  let byday: string | null = null;
  let bymonthday: string | null = null;
  for (const [k, v] of parts) {
    if (seen.has(k)) return `The rule names ${k} twice.`;
    seen.add(k);
    switch (k) {
      case "FREQ": {
        const f = FREQ_OF[v];
        if (!f) return `The builder has no ${v.toLowerCase()} frequency.`;
        m.freq = f;
        break;
      }
      case "INTERVAL": {
        const n = Number(v);
        if (!/^\d+$/.test(v) || n < 1) return `INTERVAL=${v} is not a whole number of at least 1.`;
        m.interval = n;
        break;
      }
      case "COUNT": {
        const n = Number(v);
        if (!/^\d+$/.test(v) || n < 1) return `COUNT=${v} is not a whole number of at least 1.`;
        m.end = { kind: "count", count: n };
        break;
      }
      case "UNTIL": {
        const d = untilDate(v);
        if (!d) return `UNTIL=${v} is not a date or a UTC time.`;
        m.end = { kind: "until", date: d };
        break;
      }
      case "BYDAY":
        byday = v;
        break;
      case "BYMONTHDAY":
        bymonthday = v;
        break;
      default:
        return `The builder cannot show ${k}.`;
    }
  }
  if (!seen.has("FREQ")) return "The rule has no FREQ.";
  if (seen.has("COUNT") && seen.has("UNTIL")) return "The rule has both COUNT and UNTIL.";
  if (m.freq === "weekly") {
    if (bymonthday !== null) return "The builder cannot show BYMONTHDAY on a weekly rule.";
    if (byday !== null) {
      const days = byday.split(",");
      if (!days.every(isWeekday)) return `The builder cannot show BYDAY=${byday} on a weekly rule.`;
      m.weekdays = WEEKDAYS.filter((d) => days.includes(d));
    }
  } else if (m.freq === "monthly") {
    if (byday !== null && bymonthday !== null) return "The builder cannot show both BYDAY and BYMONTHDAY.";
    if (bymonthday !== null) {
      const n = Number(bymonthday);
      if (!/^-?\d+$/.test(bymonthday) || !((n >= 1 && n <= 31) || n === -1)) return `The builder cannot show BYMONTHDAY=${bymonthday}.`;
      m.monthly = { by: "day", day: n };
    } else if (byday !== null) {
      const d = /^([+-]?\d)(MO|TU|WE|TH|FR|SA|SU)$/.exec(byday);
      const nth = d ? Number(d[1]) : 0;
      if (!d || !((nth >= 1 && nth <= 4) || nth === -1)) return `The builder cannot show BYDAY=${byday} on a monthly rule.`;
      m.monthly = { by: "weekday", nth, weekday: d[2] as Weekday };
    }
  } else if (byday !== null || bymonthday !== null) {
    return `The builder cannot show BYDAY or BYMONTHDAY on a ${m.freq} rule.`;
  }
  return m;
}

/** What the builder makes of a field's lines. */
export interface ReadRule {
  /** The builder's model, or null when the rule is raw (or empty). */
  model: RuleModel | null;
  /** Why the rule shows as raw lines, or null. */
  reason: string | null;
}

/** Read stored lines for the builder: one plain `RRULE` line, else raw with the reason. */
export function readRule(value: unknown): ReadRule {
  const lines = ruleLines(value);
  if (!lines.length) return { model: null, reason: null };
  const rrules = lines.filter((l) => property(l) === "RRULE");
  const other = lines.filter((l) => property(l) !== "RRULE");
  if (other.length) {
    const names = [...new Set(other.map(property))].join(", ");
    return { model: null, reason: `The rule has ${names} lines, which the builder cannot show.` };
  }
  if (rrules.length !== 1) return { model: null, reason: "The rule has more than one RRULE line, which the builder cannot show." };
  const m = modelOf(rrules[0]!);
  return typeof m === "string" ? { model: null, reason: m } : { model: m, reason: null };
}

/** The `RRULE` line for a model. `allDay` when the series' first occurrence is a date. */
export function ruleLine(m: RuleModel, allDay: boolean): string {
  const parts = [`FREQ=${m.freq.toUpperCase()}`];
  if (m.interval > 1) parts.push(`INTERVAL=${m.interval}`);
  if (m.freq === "weekly" && m.weekdays.length) parts.push(`BYDAY=${WEEKDAYS.filter((d) => m.weekdays.includes(d)).join(",")}`);
  if (m.freq === "monthly") {
    if (m.monthly.by === "day") parts.push(`BYMONTHDAY=${m.monthly.day}`);
    if (m.monthly.by === "weekday") parts.push(`BYDAY=${m.monthly.nth}${m.monthly.weekday}`);
  }
  if (m.end.kind === "count") parts.push(`COUNT=${m.end.count}`);
  if (m.end.kind === "until") parts.push(`UNTIL=${untilValue(m.end.date, allDay)}`);
  return `RRULE:${parts.join(";")}`;
}

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/** `2027-06-30` as "30 June 2027". */
export function longDate(date: string): string {
  const [y, m, d] = date.split("-").map(Number) as [number, number, number];
  return `${d} ${MONTHS[m - 1] ?? ""} ${y}`;
}

/** "Mon", "Mon and Wed", "Mon, Tue and Wed". */
function and(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

/** "1st" to "4th", "last". */
export function ordinal(n: number): string {
  if (n === -1) return "last";
  return `${n}${n === 1 ? "st" : n === 2 ? "nd" : n === 3 ? "rd" : "th"}`;
}

const UNIT: Record<Freq, [string, string]> = {
  daily: ["day", "days"],
  weekly: ["week", "weeks"],
  monthly: ["month", "months"],
  yearly: ["year", "years"],
};

/** A model in words: "Every 2 weeks on Mon and Wed, until 30 June 2027". */
export function modelSummary(m: RuleModel): string {
  const [one, many] = UNIT[m.freq];
  let s = m.interval > 1 ? `Every ${m.interval} ${many}` : `Every ${one}`;
  if (m.freq === "weekly" && m.weekdays.length) s += ` on ${and(WEEKDAYS.filter((d) => m.weekdays.includes(d)).map((d) => WEEKDAY_SHORT[d]))}`;
  if (m.freq === "monthly" && m.monthly.by === "day") s += m.monthly.day === -1 ? " on the last day" : ` on day ${m.monthly.day}`;
  if (m.freq === "monthly" && m.monthly.by === "weekday") s += ` on the ${ordinal(m.monthly.nth)} ${WEEKDAY_LONG[m.monthly.weekday]}`;
  if (m.end.kind === "until") s += `, until ${longDate(m.end.date)}`;
  if (m.end.kind === "count") s += m.end.count === 1 ? ", once" : `, ${m.end.count} times`;
  return s;
}

/** How many dates the lines of property `name` list. */
function dateCount(lines: string[], name: string): number {
  return lines
    .filter((l) => property(l) === name)
    .reduce((n, l) => n + l.slice(l.indexOf(":") + 1).split(",").filter((x) => x.trim()).length, 0);
}

const dates = (n: number) => (n === 1 ? "1 date" : `${n} dates`);

/**
 * Stored lines in words, raw ones too: the first `RRULE` the builder can read as its summary, a
 * custom rule as "Custom rule", with ", except 2 dates" for `EXDATE` lines and ", plus 1 date"
 * for `RDATE` lines. "" for no rule.
 */
export function ruleSummary(value: unknown): string {
  const lines = ruleLines(value);
  if (!lines.length) return "";
  const rrules = lines.filter((l) => property(l) === "RRULE");
  const m = rrules.length === 1 ? modelOf(rrules[0]!) : null;
  let s = m && typeof m !== "string" ? modelSummary(m) : rrules.length ? "Custom rule" : "On set dates";
  const ex = dateCount(lines, "EXDATE");
  const plus = dateCount(lines, "RDATE");
  if (ex) s += `, except ${dates(ex)}`;
  if (plus) s += `, plus ${dates(plus)}`;
  return s;
}

/**
 * Whether `f` is the rule of the built-in event series type, so it gets the rule editor and its
 * summary: a list of strings named `rule` alike with `event_series`'s own, or any field of that
 * name when `typeName` says it is on an `event_series` note.
 */
export function isRuleField(f: FieldDef, typeName?: string): boolean {
  if (f.name !== RULE_FIELD || f.kind !== "list" || (f.items ?? "string") !== "string") return false;
  if (typeName === SERIES_TYPE) return true;
  const own = typeDef(SERIES_TYPE)?.fields.find((x) => x.name === RULE_FIELD);
  return !!own && (own === f || (own.kind === f.kind && own.label === f.label && own.description === f.description));
}
