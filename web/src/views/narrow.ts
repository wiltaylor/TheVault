// What the kanban, calendar and timeline views need from a view's query: the query narrowed to
// one column or one date window, the field the view is built on, and each row as an item.

import type { FieldDef, Json, Row, Term, TypeDef } from "../api";
import { termColour } from "../defs/taxonomyModel";

/** The clauses that may follow `from`, in grammar order (PRD Appendix B). */
const CLAUSES = new Set(["where", "search", "sort", "group", "select", "limit"]);

/**
 * Each top-level clause word in `query` and where it starts. Strings and member names are skipped.
 * `include` (as in `include archived`) is a clause only straight after the type list, before any
 * other clause, since a field may be called `include`.
 */
function clauses(query: string): { word: string; at: number }[] {
  const out: { word: string; at: number }[] = [];
  let i = 0;
  while (i < query.length) {
    const c = query[i]!;
    if (c === '"') {
      i++;
      while (i < query.length && query[i] !== '"') i += query[i] === "\\" ? 2 : 1;
      i++;
      continue;
    }
    if (/[A-Za-z_]/.test(c)) {
      const start = i;
      while (i < query.length && /[A-Za-z0-9_]/.test(query[i]!)) i++;
      const word = query.slice(start, i);
      if ((CLAUSES.has(word) || (word === "include" && out.length === 0)) && query[start - 1] !== ".") out.push({ word, at: start });
      continue;
    }
    i++;
  }
  return out;
}

/** `query` with `condition` added to its `where` clause, so it returns only the notes that meet both. */
export function narrow(query: string, condition: string): string {
  const found = clauses(query);
  const where = found.findIndex((c) => c.word === "where");
  if (where >= 0) {
    const start = found[where]!.at + "where".length;
    const end = found[where + 1]?.at ?? query.length;
    const expr = query.slice(start, end).trim();
    const rest = query.slice(end).trim();
    return `${query.slice(0, found[where]!.at)}where (${expr}) && (${condition})${rest ? ` ${rest}` : ""}`;
  }
  // A new `where` goes after `include archived`, which follows the type list.
  const at = found.find((c) => c.word !== "include")?.at ?? query.length;
  const rest = query.slice(at).trim();
  return `${query.slice(0, at).trimEnd()} where ${condition}${rest ? ` ${rest}` : ""}`;
}

/** The types a query reads, or null for `from *`. */
export function fromTypes(query: string): string[] | null {
  const end = clauses(query)[0]?.at ?? query.length;
  const list = query.slice(0, end).trim().replace(/^from\s+/, "");
  if (list === "*") return null;
  return list
    .split(",")
    .map((t) => t.trim())
    .filter(Boolean);
}

/** A string literal in the query language. */
export function quote(s: string): string {
  return `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

export interface FieldCheck {
  field: FieldDef | null;
  /** Why the view cannot be shown, or null when it can. */
  problem: string | null;
}

/** The fields named `name` on the types `query` reads. */
function fieldsNamed(types: TypeDef[], query: string, name: string): FieldDef[] {
  const from = fromTypes(query);
  return types
    .filter((t) => from === null || from.includes(t.name))
    .flatMap((t) => t.fields.filter((f) => f.name === name));
}

/** "a select field", "an int field". */
function article(kind: string): string {
  const k = kind.replace(/_/g, " ");
  return `${/^[aeiou]/.test(k) ? "an" : "a"} ${k} field`;
}

/** The kanban's `group` field, which must be a single-term taxonomy field (D7). */
export function groupField(types: TypeDef[], query: string, name: string | null): FieldCheck {
  if (!name) {
    return { field: null, problem: "A kanban view needs a group: a single-term taxonomy field whose terms are its columns." };
  }
  const found = fieldsNamed(types, query, name);
  const f = found[0];
  if (!f) return { field: null, problem: `No type in this query has a field named ${name}.` };
  const bad = found.find((g) => g.kind !== "taxonomy");
  if (bad) {
    return {
      field: null,
      problem: `A kanban view needs a single-term taxonomy field, but ${bad.label} is ${article(bad.kind)}.`,
    };
  }
  const multi = found.find((g) => g.multiple);
  if (multi) {
    return {
      field: null,
      problem: `A kanban view needs a single-term taxonomy field, but ${multi.label} holds several terms. Choose a field that holds one term.`,
    };
  }
  return { field: f, problem: null };
}

const DATE_KINDS = ["date", "datetime", "date_range"];

/** The calendar's or timeline's `date` field: a date, datetime or date range field. */
export function dateField(types: TypeDef[], query: string, name: string | null): FieldCheck {
  if (!name) return { field: null, problem: "This view needs a date: a date, datetime or date range field." };
  const found = fieldsNamed(types, query, name);
  const f = found[0];
  if (!f) return { field: null, problem: `No type in this query has a field named ${name}.` };
  const bad = found.find((g) => !DATE_KINDS.includes(g.kind) || g.multiple);
  if (bad) {
    return {
      field: null,
      problem: `This view needs a date, datetime or date range field, but ${bad.label} is ${article(bad.kind)}.`,
    };
  }
  return { field: f, problem: null };
}

export interface KanbanColumn {
  /** The term's path, or "" for the notes with no term. */
  key: string;
  label: string;
  colour: string | null;
  /** The value a card dropped here gets: the term's path, or undefined to clear the field. */
  value: string | undefined;
  /** The condition that narrows the query to this column. */
  where: string;
}

/** One column per term of the taxonomy, in display order, then one for notes holding no term. */
export function kanbanColumns(terms: Term[], field: string, label: string): KanbanColumn[] {
  const flat: Term[] = [];
  const walk = (ts: Term[]) => {
    for (const t of ts) {
      flat.push(t);
      walk(t.children);
    }
  };
  walk(terms);
  const test = (t: Term) => `${field} == ${quote(t.path)}`;
  const cols: KanbanColumn[] = flat.map((t) => ({
    key: t.path,
    label: t.path.split("/").join(" / "),
    colour: termColour(terms, t.path) ?? null,
    value: t.path,
    where: test(t),
  }));
  // A note whose term is not in the taxonomy any more lands here too.
  cols.push({
    key: "",
    label: `No ${label.toLowerCase()}`,
    colour: null,
    value: undefined,
    where: flat.length ? `!(${flat.map(test).join(" || ")})` : "true",
  });
  return cols;
}

/** The condition for notes whose date falls in [`start`, `end`), or whose range overlaps it. */
export function dateWindow(f: FieldDef, start: string, end: string): string {
  if (f.kind === "date_range") return `${f.name}.start < ${quote(end)} && ${f.name}.end >= ${quote(start)}`;
  return `${f.name} >= ${quote(start)} && ${f.name} < ${quote(end)}`;
}

const DATE = /^\d{4}-\d{2}-\d{2}$/;

/** The day after a `YYYY-MM-DD` date: calendars and timelines end a span before its end. */
function dayAfter(date: string): string {
  const d = new Date(`${date}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + 1);
  return d.toISOString().slice(0, 10);
}

interface Span {
  start: string;
  /** Exclusive, or undefined for a single point in time. */
  end: string | undefined;
  allDay: boolean;
}

/** The time a row's date field covers, or null when the row has no usable date. */
function span(row: Row, f: FieldDef): Span | null {
  const v: Json | undefined = row.fields[f.name];
  if (f.kind === "date_range") {
    if (!v || typeof v !== "object" || Array.isArray(v)) return null;
    const { start, end } = v;
    if (typeof start !== "string" || typeof end !== "string") return null;
    const allDay = DATE.test(start);
    return { start, end: allDay ? dayAfter(end) : end, allDay };
  }
  if (typeof v !== "string") return null;
  return { start: v, end: undefined, allDay: DATE.test(v) };
}

export interface CalendarEvent {
  id: string;
  title: string;
  start: string;
  end?: string;
  allDay: boolean;
}

/** A row as a calendar event: a date on its day, a range over every day it covers. */
export function calendarEvent(row: Row, f: FieldDef): CalendarEvent | null {
  const s = span(row, f);
  if (!s) return null;
  return { id: row.id, title: row.title || "Untitled", start: s.start, ...(s.end ? { end: s.end } : {}), allDay: s.allDay };
}

export interface TimelineItem {
  id: string;
  content: string;
  start: string;
  end?: string;
  type: "point" | "range";
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

/** A row on the timeline: a date range as a range, a single date or datetime as a point. */
export function timelineItem(row: Row, f: FieldDef): TimelineItem | null {
  const s = span(row, f);
  if (!s) return null;
  const content = escapeHtml(row.title || "Untitled");
  return s.end
    ? { id: row.id, content, start: s.start, end: s.end, type: "range" }
    : { id: row.id, content, start: s.start, type: "point" };
}

function localDate(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/**
 * The condition for the notes a calendar or timeline showing [`start`, `end`) needs. Days are
 * compared as the local days on screen, times as instants.
 */
export function visibleWindow(f: FieldDef, start: Date, end: Date): string {
  const days = dateWindow(f, localDate(start), localDate(end));
  const times = dateWindow(f, start.toISOString(), end.toISOString());
  // A range of either form holds dates on some notes and datetimes on others: take the notes
  // either reading finds, and let each value's own form place it.
  if (f.kind === "date_range" && f.of === "date_or_datetime") return `(${days}) || (${times})`;
  return f.kind === "date" || (f.kind === "date_range" && f.of !== "datetime") ? days : times;
}

/** The condition for notes that hold a date in `f` at all. */
export function hasDate(f: FieldDef): string {
  return dateWindow(f, "0001-01-01", "9999-12-31");
}
