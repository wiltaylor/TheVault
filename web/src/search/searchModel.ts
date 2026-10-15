// The Search page's state: full-text, filters and sort. It lives in the page's URL query string,
// and becomes query-language text (PRD Appendix B) that runs through POST /api/query. Pure: no
// signals, no DOM, so every operator is unit-tested.

import type { ArchivedFilter, FieldDef, TypeDef } from "../api";

export type SortKey = "relevance" | "updated" | "created" | "title";
export type SortDir = "asc" | "desc";

/** The ways a filter can test a field. Which apply depends on the field's kind, see [`opsFor`]. */
export type Op =
  // text
  | "contains"
  | "is"
  | "is_not"
  | "any_of"
  // numbers
  | "eq"
  | "ne"
  | "lt"
  | "le"
  | "gt"
  | "ge"
  // bool
  | "true"
  | "false"
  // taxonomy
  | "under"
  // list
  | "has"
  // dates
  | "on"
  | "before"
  | "after"
  | "between"
  | "last"
  // any kind
  | "empty"
  | "not_empty";

/**
 * One condition on one field. `owner` is the type the field was picked from; it narrows the
 * search to that type when no type filter is set. `values` holds what the operator needs: one
 * value, two dates for `between`, a day count for `last`, or the options of `any_of`.
 */
export interface Filter {
  /** `date` filters come from the Date section, `prop` from Properties. Both test a field. */
  section: "date" | "prop";
  owner: string | null;
  field: string;
  op: Op;
  values: string[];
}

export interface SearchState {
  text: string;
  types: string[];
  filters: Filter[];
  /** Null for the default: relevance with text, else updated newest first. */
  sort: SortKey | null;
  /** Null for the sort's own default direction. */
  dir: SortDir | null;
  /** Archived notes: left out (the default), included with the rest, or the only ones. */
  archived: ArchivedFilter;
}

export const EMPTY: SearchState = { text: "", types: [], filters: [], sort: null, dir: null, archived: "exclude" };

/** The archived filter's choices, in menu order. */
export const ARCHIVED_CHOICES: [ArchivedFilter, string][] = [
  ["exclude", "Hide archived"],
  ["include", "Include archived"],
  ["only", "Only archived"],
];

/** Fields every note has. */
export const BUILTIN_DATES: { name: string; label: string }[] = [
  { name: "created", label: "Created" },
  { name: "updated", label: "Updated" },
];

const BUILTIN_DEF = (name: string): FieldDef => ({
  name,
  label: name === "created" ? "Created" : "Updated",
  description: null,
  kind: "datetime",
  required: true,
  computed: true,
  multiple: false,
  compound: false,
});

/** Kinds that hold no value a filter can test beyond being empty. */
const SKIPPED = new Set(["content", "ink", "canvas"]);

export const DATE_KINDS = new Set(["date", "datetime", "date_range"]);

export function isDateField(f: FieldDef): boolean {
  return DATE_KINDS.has(f.kind);
}

/** Whether a property filter can be added on `f`. */
export function filterable(f: FieldDef): boolean {
  return !SKIPPED.has(f.kind) && f.name !== "title";
}

/** The operators a field offers, with their labels, in menu order. */
export function opsFor(f: FieldDef): [Op, string][] {
  const empties: [Op, string][] = [
    ["empty", "is empty"],
    ["not_empty", "is not empty"],
  ];
  const dates: [Op, string][] = [
    ["on", "on"],
    ["before", "before"],
    ["after", "after"],
    ["between", "between"],
    ["last", "in the last"],
  ];
  const builtin = f.name === "created" || f.name === "updated";
  switch (f.kind) {
    case "string":
    case "url":
      return [["contains", "contains"], ["is", "is"], ["is_not", "is not"], ...empties];
    case "int":
    case "float":
      return [["eq", "="], ["ne", "≠"], ["lt", "<"], ["le", "≤"], ["gt", ">"], ["ge", "≥"], ...empties];
    case "bool":
      return [
        ["true", "is true"],
        ["false", "is false"],
      ];
    case "select":
      return [["is", "is"], ["is_not", "is not"], ["any_of", "is any of"], ...empties];
    case "taxonomy":
      return [["is", "is"], ["under", "is under"], ...empties];
    case "reference":
      return [["is", "is"], ...empties];
    case "list":
      return [["has", "contains"], ...empties];
    case "date":
    case "datetime":
    case "date_range":
      return builtin ? dates : [...dates, ...empties];
    default:
      return empties;
  }
}

/** How many values `op` takes from its inputs. `any_of` takes one or more. */
export function arity(op: Op): number {
  switch (op) {
    case "true":
    case "false":
    case "empty":
    case "not_empty":
      return 0;
    case "between":
      return 2;
    default:
      return 1;
  }
}

/** The operator a new filter on `f` starts with. */
export function defaultOp(f: FieldDef): Op {
  return opsFor(f)[0]![0];
}

/** A fresh filter on `field` of `owner`, from `section`. */
export function newFilter(section: Filter["section"], owner: string | null, f: FieldDef): Filter {
  const op = section === "date" ? "last" : defaultOp(f);
  return { section, owner, field: f.name, op, values: op === "last" ? ["7"] : [] };
}

/** `f` moved to operator `op`, keeping the values that still make sense. */
export function withOp(f: Filter, op: Op): Filter {
  const dateOps: Op[] = ["on", "before", "after", "between"];
  let values = f.values;
  if (arity(op) === 0) values = [];
  else if (op === "last") values = f.op === "last" ? f.values : ["7"];
  else if (dateOps.includes(op) !== dateOps.includes(f.op) || f.op === "last") values = [];
  else if (op !== "any_of" && op !== "between") values = values.slice(0, 1);
  return { ...f, op, values };
}

// ---------------------------------------------------------------------------------------------
// Fields

/** The fields a filter may test: those of the chosen types, or of every type when none is. */
export function fieldOf(types: TypeDef[], owner: string | null, chosen: string[], name: string): FieldDef | undefined {
  if (name === "created" || name === "updated") return BUILTIN_DEF(name);
  const where = chosen.length ? chosen : owner ? [owner] : [];
  for (const t of where) {
    const f = types.find((x) => x.name === t)?.fields.find((x) => x.name === name);
    if (f) return f;
  }
  return undefined;
}

/** The type names the fields a filter may test come from, for the pickers. */
export function fieldGroups(
  types: TypeDef[],
  chosen: string[],
  want: (f: FieldDef) => boolean,
): { type: TypeDef | null; fields: FieldDef[] }[] {
  if (chosen.length) {
    // One list: every field of the chosen types, each name once.
    const seen = new Set<string>();
    const fields: FieldDef[] = [];
    for (const name of chosen) {
      for (const f of types.find((t) => t.name === name)?.fields ?? []) {
        if (want(f) && !seen.has(f.name)) {
          seen.add(f.name);
          fields.push(f);
        }
      }
    }
    return [{ type: null, fields }];
  }
  return types
    .map((t) => ({ type: t as TypeDef | null, fields: t.fields.filter(want) }))
    .filter((g) => g.fields.length > 0);
}

// ---------------------------------------------------------------------------------------------
// Query text

/** A wscript string literal, escaped as the query printer escapes it. */
export function lit(s: string): string {
  let out = '"';
  for (const c of s) {
    switch (c) {
      case '"':
        out += '\\"';
        break;
      case "\\":
        out += "\\\\";
        break;
      case "\n":
        out += "\\n";
        break;
      case "\t":
        out += "\\t";
        break;
      case "\r":
        out += "\\r";
        break;
      case "\0":
        out += "\\0";
        break;
      case "{":
        out += "{{";
        break;
      case "}":
        out += "}}";
        break;
      default: {
        const code = c.codePointAt(0)!;
        out += code < 0x20 || code === 0x7f ? `\\u{${code.toString(16)}}` : c;
      }
    }
  }
  return `${out}"`;
}

/** A number literal, or null when `s` is not a number. */
function num(s: string | undefined): string | null {
  const t = (s ?? "").trim();
  if (t === "" || !Number.isFinite(Number(t))) return null;
  return /^-?\d+$/.test(t) ? String(BigInt(t)) : String(Number(t));
}

const DAY = /^\d{4}-\d\d-\d\d$/;

/** The day after `YYYY-MM-DD`. */
export function nextDay(d: string): string {
  const [y, m, dd] = d.split("-").map(Number) as [number, number, number];
  const t = new Date(Date.UTC(y, m - 1, dd + 1));
  return t.toISOString().slice(0, 10);
}

/** Minutes ahead of UTC at local midnight on `day`, as `Date` would report for this browser. */
export type OffsetAt = (day: string) => number;

export const browserOffset: OffsetAt = (day) => {
  const [y, m, d] = day.split("-").map(Number) as [number, number, number];
  return -new Date(y, m - 1, d).getTimezoneOffset();
};

/** Local midnight at the start of `day`, as a datetime literal. */
function midnight(day: string, offset: OffsetAt): string {
  const minutes = offset(day);
  if (minutes === 0) return lit(`${day}T00:00:00Z`);
  const abs = Math.abs(minutes);
  const pad = (n: number) => String(n).padStart(2, "0");
  return lit(`${day}T00:00:00${minutes < 0 ? "-" : "+"}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`);
}

/** A condition joined into an `&&` chain: anything with `||` in it goes in parentheses. */
const term = (c: string) => (/\|\|/.test(c) ? `(${c})` : c);
const all = (cs: string[]) => cs.map(term).join(" && ");
const anyOf = (cs: string[]) => (cs.length === 1 ? cs[0]! : cs.join(" || "));

/**
 * A date condition on `subject`: a field whose values are days (`date`) or instants. Days compare
 * as days; an instant is on a day from that day's local midnight up to the next one.
 */
function dateCond(subject: string, days: boolean, op: Op, v: string[], offset: OffsetAt): string | null {
  const [a, b] = v;
  if (op === "last") {
    const n = num(a);
    if (n === null || !/^\d+$/.test(n) || n === "0") return null;
    return days
      ? `${subject} >= today() - days(${n}) && ${subject} <= today()`
      : `${subject} >= now() - days(${n})`;
  }
  if (!a || !DAY.test(a)) return null;
  const at = (d: string) => (days ? lit(d) : midnight(d, offset));
  switch (op) {
    case "on":
      return days ? `${subject} == ${lit(a)}` : `${subject} >= ${at(a)} && ${subject} < ${at(nextDay(a))}`;
    case "before":
      return `${subject} < ${at(a)}`;
    case "after":
      return days ? `${subject} > ${lit(a)}` : `${subject} >= ${at(nextDay(a))}`;
    case "between": {
      if (!b || !DAY.test(b)) return null;
      const [lo, hi] = a <= b ? [a, b] : [b, a];
      return days
        ? `${subject} >= ${lit(lo)} && ${subject} <= ${lit(hi)}`
        : `${subject} >= ${at(lo)} && ${subject} < ${at(nextDay(hi))}`;
    }
    default:
      return null;
  }
}

/** A date range condition: the range overlaps the days asked for. */
function rangeCond(f: string, days: boolean, op: Op, v: string[], offset: OffsetAt): string | null {
  const [s, e] = [`${f}.start`, `${f}.end`];
  const [a, b] = v;
  if (op === "last") {
    const n = num(a);
    if (n === null || !/^\d+$/.test(n) || n === "0") return null;
    return days
      ? `${s} <= today() && ${e} >= today() - days(${n})`
      : `${s} <= now() && ${e} >= now() - days(${n})`;
  }
  if (!a || !DAY.test(a)) return null;
  const at = (d: string) => (days ? lit(d) : midnight(d, offset));
  switch (op) {
    case "on":
      return days ? `${s} <= ${lit(a)} && ${e} >= ${lit(a)}` : `${s} < ${at(nextDay(a))} && ${e} >= ${at(a)}`;
    case "before":
      return `${s} < ${at(a)}`;
    case "after":
      return days ? `${e} > ${lit(a)}` : `${e} >= ${at(nextDay(a))}`;
    case "between": {
      if (!b || !DAY.test(b)) return null;
      const [lo, hi] = a <= b ? [a, b] : [b, a];
      return days ? `${s} <= ${lit(hi)} && ${e} >= ${lit(lo)}` : `${s} < ${at(nextDay(hi))} && ${e} >= ${at(lo)}`;
    }
    default:
      return null;
  }
}

const COMPARE: Partial<Record<Op, string>> = { eq: "==", ne: "!=", lt: "<", le: "<=", gt: ">", ge: ">=" };

/** A list item literal of the list's item kind. */
function itemLit(items: string | undefined, v: string): string | null {
  if (items === "int" || items === "float") return num(v);
  if (items === "bool") return v === "true" || v === "false" ? v : null;
  return lit(v);
}

/**
 * The condition one filter adds, or null while it is incomplete (a value not given yet). A field
 * that holds several values is tested with `has()` for equality and `any()` otherwise.
 */
export function filterCond(def: FieldDef, filter: Filter, offset: OffsetAt = browserOffset): string | null {
  const f = filter.field;
  const v = filter.values;
  const one = v[0] ?? "";
  const many = def.multiple;
  const is = (x: string) => (many ? `has(${f}, ${x})` : `${f} == ${x}`);
  switch (filter.op) {
    case "empty":
      return `empty(${f})`;
    case "not_empty":
      return `!empty(${f})`;
    case "true":
      return many ? `has(${f}, true)` : f;
    case "false":
      return many ? `!has(${f}, true)` : `!${f}`;
    case "contains":
      return one.trim() ? `contains(${f}, ${lit(one)})` : null;
    case "is":
      return one === "" ? null : is(lit(one));
    case "is_not":
      return one === "" ? null : many ? `!has(${f}, ${lit(one)})` : `${f} != ${lit(one)}`;
    case "any_of": {
      const chosen = v.filter((x) => x !== "");
      return chosen.length ? anyOf(chosen.map((x) => is(lit(x)))) : null;
    }
    case "under":
      return one === "" ? null : `under(${f}, ${lit(one)})`;
    case "has": {
      const x = itemLit(def.items, one);
      return one === "" || x === null ? null : `has(${f}, ${x})`;
    }
    case "eq":
    case "ne":
    case "lt":
    case "le":
    case "gt":
    case "ge": {
      const n = num(one);
      if (n === null) return null;
      const op = COMPARE[filter.op]!;
      return many ? `any(${f}, |v| v ${op} ${n})` : `${f} ${op} ${n}`;
    }
    case "on":
    case "before":
    case "after":
    case "between":
    case "last": {
      if (def.kind === "date_range") return rangeCond(f, def.of !== "datetime", filter.op, v, offset);
      const days = def.kind === "date";
      if (many) {
        const c = dateCond("v", days, filter.op, v, offset);
        return c && `any(${f}, |v| ${c})`;
      }
      return dateCond(f, days, filter.op, v, offset);
    }
  }
}

/** The types the query reads: the chosen ones, else the owners of the filters, else every type. */
export function queryTypes(s: SearchState): string[] {
  if (s.types.length) return s.types;
  const owners: string[] = [];
  for (const f of s.filters) if (f.owner && !owners.includes(f.owner)) owners.push(f.owner);
  return owners;
}

/** The sort the query uses, resolved from the defaults. */
export function effectiveSort(s: SearchState): { key: SortKey; dir: SortDir } {
  const key = s.sort ?? (s.text.trim() ? "relevance" : "updated");
  const resolved: SortKey = key === "relevance" && !s.text.trim() ? "updated" : key;
  const dir = s.dir ?? (resolved === "title" ? "asc" : "desc");
  return { key: resolved, dir };
}

/**
 * The query text of a search. Incomplete filters are left out. With no type chosen, a filter on
 * a field of one type narrows the query to that type; filters on fields of several types each
 * test their own type.
 */
export function toQuery(s: SearchState, types: TypeDef[], offset: OffsetAt = browserOffset): string {
  const from = queryTypes(s);
  const conds: string[] = [];
  const guard = !s.types.length && from.length > 1;
  for (const filter of s.filters) {
    const def = fieldOf(types, filter.owner, s.types, filter.field);
    if (!def) continue;
    const c = filterCond(def, filter, offset);
    if (c === null) continue;
    conds.push(guard && filter.owner ? all([`type == ${lit(filter.owner)}`, c]) : c);
  }
  // Archived notes are left out unless asked for: `include archived` keeps them, a condition on
  // `archived` reads nothing else.
  if (s.archived === "only") conds.push("archived");
  let q = `from ${from.length ? from.join(", ") : "*"}`;
  if (s.archived === "include") q += " include archived";
  if (conds.length) q += ` where ${all(conds)}`;
  // The last word matches as a prefix while it is being typed; a space after it says it is
  // complete, so one trailing space is kept.
  const text = s.text.trim();
  if (text) q += ` search ${lit(/\s$/.test(s.text) ? `${text} ` : text)}`;
  const { key, dir } = effectiveSort(s);
  if (key !== "relevance") q += ` sort ${key} ${dir}`;
  return q;
}

/** How many filters the page shows: the type filter counts once, and so does asking for archived notes. */
export function filterCount(s: SearchState): number {
  return (s.types.length ? 1 : 0) + s.filters.length + (s.archived === "exclude" ? 0 : 1);
}

// ---------------------------------------------------------------------------------------------
// URL

const OPS = new Set<Op>([
  "contains", "is", "is_not", "any_of", "eq", "ne", "lt", "le", "gt", "ge", "true", "false",
  "under", "has", "on", "before", "after", "between", "last", "empty", "not_empty",
]);
const SORTS = new Set<SortKey>(["relevance", "updated", "created", "title"]);

/** A URL query part, with `~` escaped too, since it separates a filter's parts. */
const enc = (s: string) => encodeURIComponent(s).replace(/~/g, "%7E");

function dec(s: string): string | null {
  try {
    return decodeURIComponent(s.replace(/\+/g, " "));
  } catch {
    return null;
  }
}

/** One filter as a URL parameter value: its owner, field, operator and values, joined by `~`. */
function encodeFilter(f: Filter): string {
  return [f.owner ?? "", f.field, f.op, ...f.values].map(enc).join("~");
}

function decodeFilter(section: Filter["section"], raw: string): Filter | null {
  const parts = raw.split("~").map(dec);
  if (parts.some((p) => p === null)) return null;
  const [owner, field, op, ...values] = parts as string[];
  if (!field || !op || !OPS.has(op as Op)) return null;
  return { section, owner: owner || null, field, op: op as Op, values };
}

/**
 * The query string of a search, without `?`: `q`, then `type`, `date` and `prop` once per type
 * or filter, then `archived` when archived notes are asked for, then `sort` and `dir`. The
 * default search is the empty string.
 */
export function encodeSearch(s: SearchState): string {
  const p: string[] = [];
  if (s.text) p.push(`q=${enc(s.text)}`);
  for (const t of s.types) p.push(`type=${enc(t)}`);
  for (const f of s.filters) p.push(`${f.section === "date" ? "date" : "prop"}=${encodeFilter(f)}`);
  if (s.archived !== "exclude") p.push(`archived=${s.archived}`);
  if (s.sort) p.push(`sort=${s.sort}`);
  if (s.dir) p.push(`dir=${s.dir}`);
  return p.join("&");
}

/** A search from a query string, with or without `?`. Anything it cannot read is dropped. */
export function decodeSearch(query: string): SearchState {
  const out: SearchState = { text: "", types: [], filters: [], sort: null, dir: null, archived: "exclude" };
  const qs = query.startsWith("?") ? query.slice(1) : query;
  for (const pair of qs.split("&")) {
    if (!pair) continue;
    const at = pair.indexOf("=");
    const key = dec(at < 0 ? pair : pair.slice(0, at));
    const raw = at < 0 ? "" : pair.slice(at + 1);
    if (key === "date" || key === "prop") {
      // Filters keep the order they were added in, across both sections.
      const f = decodeFilter(key, raw);
      if (f) out.filters.push(f);
      continue;
    }
    const value = dec(raw);
    if (value === null) continue;
    if (key === "q") out.text = value;
    else if (key === "type" && value) out.types.push(value);
    else if (key === "sort" && SORTS.has(value as SortKey)) out.sort = value as SortKey;
    else if (key === "dir" && (value === "asc" || value === "desc")) out.dir = value;
    else if (key === "archived" && (value === "include" || value === "only")) out.archived = value;
  }
  return out;
}

/** The search without the filters that are not complete yet, which the address leaves out. */
export function completeOnly(s: SearchState, types: TypeDef[]): SearchState {
  const filters = s.filters.filter((f) => {
    const def = fieldOf(types, f.owner, s.types, f.field);
    return !def || filterCond(def, f, () => 0) !== null;
  });
  return filters.length === s.filters.length ? s : { ...s, filters };
}

// ---------------------------------------------------------------------------------------------
// Labels

/** A short reading of a filter for its chip, such as `Due in the last 7 days`. */
export function describe(def: FieldDef | undefined, f: Filter, valueText: (v: string) => string = (v) => v): string {
  const label = def?.label ?? f.field;
  const ops = def ? opsFor(def) : [];
  const opLabel = ops.find(([o]) => o === f.op)?.[1] ?? f.op;
  const vals = f.values.filter((v) => v !== "");
  switch (f.op) {
    case "between":
      return `${label} between ${vals[0] ?? "…"} and ${vals[1] ?? "…"}`;
    case "last":
      return `${label} in the last ${vals[0] ?? "…"} days`;
    case "any_of":
      return `${label} is any of ${vals.length ? vals.map(valueText).join(", ") : "…"}`;
    default:
      return arity(f.op) === 0 ? `${label} ${opLabel}` : `${label} ${opLabel} ${vals.length ? valueText(vals[0]!) : "…"}`;
  }
}
