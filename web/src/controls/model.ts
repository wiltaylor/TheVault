// `control` blocks (docs/vault-format.md, "control"): a form control bound to one field of the
// note it shows on. Pure helpers the renderer, the editor, the note page and the tests share: the
// fit table, a control's items, and how each control kind turns a gesture into a field value.

import type { Block, FieldDef, Json, NoteDoc, TypeDef } from "../api";

export const CONTROL_KINDS = [
  "text",
  "textarea",
  "number",
  "slider",
  "toggle",
  "checkbox",
  "dropdown",
  "radio",
  "chips",
  "calendar",
  "datetime",
  "time",
  "rating",
] as const;

export type ControlKind = (typeof CONTROL_KINDS)[number];

export function isControlKind(s: unknown): s is ControlKind {
  return typeof s === "string" && (CONTROL_KINDS as readonly string[]).includes(s);
}

/** How the `/` menu and the settings strip name each kind, and the fields it fits, in words. */
export const CONTROL_INFO: Record<ControlKind, { label: string; keywords: string; fits: string }> = {
  text: { label: "Text box", keywords: "input line field", fits: "string, url, int or float" },
  textarea: { label: "Text area", keywords: "multiline paragraph long", fits: "string" },
  number: { label: "Number stepper", keywords: "spin integer count", fits: "int or float" },
  slider: { label: "Slider", keywords: "range scale", fits: "int or float" },
  toggle: { label: "Toggle", keywords: "switch on off bool", fits: "bool" },
  checkbox: { label: "Checkbox", keywords: "tick check bool", fits: "bool" },
  dropdown: { label: "Dropdown", keywords: "select choose menu combo", fits: "single select, taxonomy or reference" },
  radio: { label: "Radio buttons", keywords: "option choose one", fits: "single select" },
  chips: { label: "Chip list", keywords: "tags multi pills tokens", fits: "list of strings, multiple select, taxonomy or reference" },
  calendar: { label: "Calendar", keywords: "date month day range", fits: "date or date range" },
  datetime: { label: "Date and time", keywords: "datetime timestamp when", fits: "datetime" },
  time: { label: "Time", keywords: "clock hour minute", fits: "datetime" },
  rating: { label: "Rating", keywords: "stars score", fits: "int" },
};

/** A rating's number of stars when it names none, and the most it may have. */
export const RATING_MAX_DEFAULT = 5;
export const RATING_MAX_LIMIT = 10;

const CHOICE_KINDS = new Set(["select", "taxonomy", "reference"]);

/** Whether a control of `kind` can show and edit field `f`. The server checks the same table. */
export function controlFits(kind: ControlKind, f: FieldDef): boolean {
  const choice = CHOICE_KINDS.has(f.kind);
  switch (kind) {
    case "text":
      return ["string", "url", "int", "float"].includes(f.kind);
    case "textarea":
      return f.kind === "string";
    case "number":
    case "slider":
      return f.kind === "int" || f.kind === "float";
    case "toggle":
    case "checkbox":
      return f.kind === "bool";
    case "dropdown":
      return choice && !f.multiple;
    case "radio":
      return f.kind === "select" && !f.multiple;
    case "chips":
      return (choice && f.multiple) || (f.kind === "list" && (f.items ?? "string") === "string");
    case "calendar":
      return f.kind === "date" || (f.kind === "date_range" && (f.of ?? "date") === "date");
    case "datetime":
    case "time":
      return f.kind === "datetime";
    case "rating":
      return f.kind === "int";
  }
}

/** The fields of `fields` a control of `kind` fits, in their order. */
export function fittingFields(kind: ControlKind, fields: readonly FieldDef[]): FieldDef[] {
  return fields.filter((f) => controlFits(kind, f));
}

/** A field's kind as the field pickers show it: `multiple select`, `list of string`. */
export function fieldKindText(f: FieldDef): string {
  if (f.kind === "list") return `list of ${f.items ?? "string"}`;
  if (f.kind === "date_range") {
    return f.of === "datetime" ? "datetime range" : f.of === "date_or_datetime" ? "date or datetime range" : "date range";
  }
  return `${f.multiple ? "multiple " : ""}${f.kind}${f.computed ? ", computed" : ""}`;
}

/** A control block's items, read leniently: what the renderer needs. */
export interface ControlSpec {
  field: string;
  kind: ControlKind;
  label: string | null;
  placeholder: string | null;
  min?: number;
  max?: number;
  step?: number;
}

const num = (v: Json | undefined): number | undefined => (typeof v === "number" && Number.isFinite(v) ? v : undefined);
const str = (v: Json | undefined): string | null => (typeof v === "string" && v !== "" ? v : null);

/** The control a `control` block describes, or null when it names no field or kind. */
export function controlOf(block: Block): ControlSpec | null {
  if (block.kind !== "control") return null;
  const f = block.fields ?? {};
  const field = str(f.field);
  if (!field || !isControlKind(f.kind)) return null;
  return {
    field,
    kind: f.kind,
    label: str(f.label),
    placeholder: str(f.placeholder),
    min: num(f.min),
    max: num(f.max),
    step: num(f.step),
  };
}

/** The items beyond field, kind and label that each kind takes. */
export function controlItems(kind: ControlKind): ("placeholder" | "min" | "max" | "step")[] {
  switch (kind) {
    case "text":
    case "textarea":
    case "dropdown":
    case "chips":
      return ["placeholder"];
    case "number":
      return ["placeholder", "min", "max", "step"];
    case "slider":
      return ["min", "max", "step"];
    case "rating":
      return ["max"];
    default:
      return [];
  }
}

/** The order items are written in, as docs/vault-format.md lists them. */
const ITEM_ORDER = ["field", "kind", "label", "placeholder", "min", "max", "step"];

/** `block` with item `name` set to `value`, or removed when `value` is undefined. */
export function withControlItem(block: Block, name: string, value: Json | undefined): Block {
  const fields: Record<string, Json> = { ...(block.fields ?? {}) };
  if (value === undefined) delete fields[name];
  else fields[name] = value;
  const ordered: Record<string, Json> = {};
  for (const k of ITEM_ORDER) if (fields[k] !== undefined) ordered[k] = fields[k]!;
  for (const [k, v] of Object.entries(fields)) if (!(k in ordered)) ordered[k] = v;
  return { ...block, fields: ordered };
}

/** A new control of `kind` bound to `field`, with the items it needs: a slider's ends. */
export function newControl(kind: ControlKind, field: string): Block {
  const fields: Record<string, Json> = { field, kind };
  if (kind === "slider") Object.assign(fields, { min: 0, max: 100, step: 1 });
  return { kind: "control", fields };
}

/** What is wrong with a control's own items, for the settings strip. Empty when nothing is. */
export function controlProblems(spec: ControlSpec, f: FieldDef | undefined): string[] {
  const out: string[] = [];
  if (!f) out.push(`The type has no field ${spec.field}.`);
  else if (!controlFits(spec.kind, f)) out.push(`A ${CONTROL_INFO[spec.kind].label.toLowerCase()} does not fit ${f.label}, a ${fieldKindText(f)} field.`);
  if (spec.kind === "slider" && (spec.min === undefined || spec.max === undefined)) out.push("A slider needs a minimum and a maximum.");
  if (spec.kind !== "rating" && spec.min !== undefined && spec.max !== undefined && spec.min >= spec.max) {
    out.push("The maximum must be greater than the minimum.");
  }
  if (spec.step !== undefined && spec.step <= 0) out.push("The step must be greater than 0.");
  if (spec.kind === "rating" && spec.max !== undefined && !(Number.isInteger(spec.max) && spec.max >= 1 && spec.max <= RATING_MAX_LIMIT)) {
    out.push(`A rating has 1 to ${RATING_MAX_LIMIT} stars.`);
  }
  if (f?.kind === "int" && spec.kind !== "rating") {
    for (const n of ["min", "max", "step"] as const) {
      const v = spec[n];
      if (v !== undefined && !Number.isInteger(v)) out.push(`The ${n} must be a whole number for an int field.`);
    }
  }
  return out;
}

// ------------------------------------------------------------------------------------------
// Values

/** True when `v` is no value at all: missing, empty text or an empty list. */
export function blank(v: Json | undefined): boolean {
  return v === undefined || v === null || v === "" || (Array.isArray(v) && v.length === 0);
}

/** Why `next` cannot be written to `f`, or null when it can: a required field cannot be emptied. */
export function refusal(f: FieldDef, label: string, next: Json | undefined): string | null {
  return f.required && blank(next) ? `${label} is required, so it cannot be empty.` : null;
}

/** Typed text as a number for an int or float field: undefined when empty, null when not a number. */
export function parseNumber(raw: string, int: boolean): number | undefined | null {
  const t = raw.trim();
  if (t === "") return undefined;
  const n = Number(t);
  if (!Number.isFinite(n)) return null;
  if (int && !Number.isInteger(n)) return null;
  return n;
}

/** `n` kept within the control's ends, when it has them. */
export function clampNumber(n: number, min?: number, max?: number): number {
  let v = n;
  if (min !== undefined && v < min) v = min;
  if (max !== undefined && v > max) v = max;
  return v;
}

/**
 * The value one step up (`dir` 1) or down (-1) from `v`, on the step grid from `min` (or 0),
 * within the ends.
 */
export function stepNumber(v: Json | undefined, dir: 1 | -1, spec: { min?: number; max?: number; step?: number }, int: boolean): number {
  const step = spec.step && spec.step > 0 ? spec.step : 1;
  const base = spec.min ?? 0;
  // From no value, up starts at the low end and down at the high end.
  if (typeof v !== "number") return clampNumber(dir > 0 ? base : (spec.max ?? base), spec.min, spec.max);
  const current = v;
  // Off the grid, a step goes to the next grid point in its direction.
  const k = (current - base) / step;
  const onGrid = Math.abs(k - Math.round(k)) < 1e-9;
  const nextK = onGrid ? Math.round(k) + dir : dir > 0 ? Math.ceil(k) : Math.floor(k);
  let next = base + nextK * step;
  next = Number(next.toFixed(10));
  if (int) next = Math.round(next);
  return clampNumber(next, spec.min, spec.max);
}

/** The value a rating takes when star `star` is chosen: that many, or none when it is the current value. */
export function ratingValue(current: Json | undefined, star: number): number | undefined {
  return current === star ? undefined : star;
}

/** A rating's value after an arrow key, Home or End, from 1 to `max`. */
export function ratingKey(current: Json | undefined, key: string, max: number): number | null {
  const v = typeof current === "number" ? current : 0;
  switch (key) {
    case "ArrowRight":
    case "ArrowUp":
      return Math.min(max, v + 1);
    case "ArrowLeft":
    case "ArrowDown":
      return Math.max(1, v - 1);
    case "Home":
      return 1;
    case "End":
      return max;
    default:
      return null;
  }
}

/** The number of stars a rating control shows. */
export function ratingMax(spec: { max?: number }): number {
  const m = spec.max;
  return m !== undefined && Number.isInteger(m) && m >= 1 && m <= RATING_MAX_LIMIT ? m : RATING_MAX_DEFAULT;
}

const asStrings = (v: Json | undefined): string[] =>
  (Array.isArray(v) ? v : v === undefined || v === null ? [] : [v]).filter((x): x is string => typeof x === "string");

/** A chip list's value with `item` added at the end; an item it holds already is not added twice. */
export function chipsAdd(value: Json | undefined, item: string): string[] {
  const now = asStrings(value);
  const t = item.trim();
  return t === "" || now.includes(t) ? now : [...now, t];
}

/** A chip list's value without `item`; no chips left is no value. */
export function chipsRemove(value: Json | undefined, item: string): string[] | undefined {
  const rest = asStrings(value).filter((x) => x !== item);
  return rest.length ? rest : undefined;
}

export function chipsOf(value: Json | undefined): string[] {
  return asStrings(value);
}

const DATETIME = /^(\d{4}-\d\d-\d\d)T(\d\d:\d\d)(:\d\d)?(\.\d+)?(Z|[+-]\d\d:\d\d)$/;

const pad = (n: number) => String(n).padStart(2, "0");

/** A local date as `YYYY-MM-DD`. */
export function isoDate(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** The offset of local time at `local` (`YYYY-MM-DDTHH:MM:SS`), as `Z` or `+HH:MM`. */
export function localOffset(local: string): string {
  const minutes = -new Date(local).getTimezoneOffset();
  if (minutes === 0 || Number.isNaN(minutes)) return "Z";
  const abs = Math.abs(minutes);
  return `${minutes < 0 ? "-" : "+"}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`;
}

/** The time part of a stored datetime, as a time input shows it: `HH:MM`, empty when there is none. */
export function timeOf(v: Json | undefined): string {
  if (typeof v !== "string") return "";
  const m = DATETIME.exec(v);
  return m ? m[2]! : "";
}

/**
 * A datetime with its time of day set to `time` (`HH:MM` or `HH:MM:SS`): the date and the offset
 * stay as they were, and an empty datetime takes today's date and the local offset. Empty `time`
 * is no value.
 */
export function withTime(previous: Json | undefined, time: string, now: Date = new Date()): string | undefined {
  if (!/^\d\d:\d\d(:\d\d)?$/.test(time)) return undefined;
  const hms = time.length === 5 ? `${time}:00` : time;
  const m = typeof previous === "string" ? DATETIME.exec(previous) : null;
  if (m) return `${m[1]}T${hms}${m[5]}`;
  const local = `${isoDate(now)}T${hms}`;
  return `${local}${localOffset(local)}`;
}

/** Add `n` days to an ISO date. */
export function addDays(iso: string, n: number): string {
  const [y, m, d] = iso.split("-").map(Number) as [number, number, number];
  return isoDate(new Date(y, m - 1, d + n));
}

/** The first of the month `n` months from the month of `iso`. */
export function addMonths(iso: string, n: number): string {
  const [y, m] = iso.split("-").map(Number) as [number, number];
  return isoDate(new Date(y, m - 1 + n, 1));
}

/** The same day `n` months on, or the month's last day when it is shorter. */
export function sameDayMonths(iso: string, n: number): string {
  const [y, m, d] = iso.split("-").map(Number) as [number, number, number];
  const last = new Date(y, m - 1 + n + 1, 0).getDate();
  return isoDate(new Date(y, m - 1 + n, Math.min(d, last)));
}

/**
 * The weeks a month calendar shows for the month of `iso`, Monday first: each a row of seven ISO
 * dates, with the days of the months either side filling the first and last rows.
 */
export function monthGrid(iso: string): string[][] {
  const [y, m] = iso.split("-").map(Number) as [number, number];
  const first = new Date(y, m - 1, 1);
  const lead = (first.getDay() + 6) % 7;
  const start = isoDate(new Date(y, m - 1, 1 - lead));
  const days = new Date(y, m, 0).getDate();
  const rows = Math.ceil((lead + days) / 7);
  return Array.from({ length: rows }, (_, r) => Array.from({ length: 7 }, (_, c) => addDays(start, r * 7 + c)));
}

/** A date range value's ends, when it is one. */
export function rangeOf(v: Json | undefined): { start: string; end: string } | null {
  if (!v || typeof v !== "object" || Array.isArray(v)) return null;
  const { start, end } = v as Record<string, Json>;
  return typeof start === "string" && typeof end === "string" ? { start, end } : null;
}

/**
 * A click on `day` in a calendar. A date field takes the day. A range takes two clicks: the first
 * marks the start and writes nothing (`pending`), the second writes the range from the earlier day
 * to the later one.
 */
export function calendarPick(
  range: boolean,
  pending: string | null,
  day: string,
): { write: true; value: Json; pending: null } | { write: false; pending: string } {
  if (!range) return { write: true, value: day, pending: null };
  if (pending === null) return { write: false, pending: day };
  const [start, end] = pending <= day ? [pending, day] : [day, pending];
  return { write: true, value: { start, end }, pending: null };
}

/** How a calendar marks `day`: chosen, an end of the range, inside it, or none of these. */
export function dayState(value: Json | undefined, pending: string | null, day: string): "selected" | "start" | "end" | "inside" | null {
  if (pending !== null) return day === pending ? "start" : null;
  if (typeof value === "string") return value === day ? "selected" : null;
  const r = rangeOf(value);
  if (!r) return null;
  if (day === r.start) return "start";
  if (day === r.end) return "end";
  return day > r.start && day < r.end ? "inside" : null;
}

// ------------------------------------------------------------------------------------------
// Placement

/** Every control in `blocks`, at any depth: in columns and in canvas cards too. */
export function controlsIn(blocks: readonly Block[] | undefined): ControlSpec[] {
  return (blocks ?? []).flatMap((b) => {
    const c = controlOf(b);
    if (c) return [c];
    if (b.kind === "slot") return [];
    return controlsIn(b.blocks);
  });
}

/** The controls a type's default template holds, on every tab and in any column. */
export function templateControls(type: TypeDef): ControlSpec[] {
  const t = type.templates.find((x) => x.name === "default") ?? type.templates[0];
  return (t?.tabs ?? []).flatMap((tab) => tab.items.flatMap((i) => ("block" in i ? controlsIn([i.block]) : [])));
}

/** The controls in a note's own content and canvas fields. */
export function noteControls(doc: NoteDoc | undefined): ControlSpec[] {
  return doc ? controlsIn(doc.blocks.filter((b) => b.kind === "content" || b.kind === "canvas")) : [];
}

/**
 * The fields of `type` an editable control places, on the note page or in the note's content: a
 * control that fits a field that is not computed. A read-only control on a computed field places
 * nothing.
 */
export function controlledFields(type: TypeDef, doc?: NoteDoc): Set<string> {
  const out = new Set<string>();
  for (const c of [...templateControls(type), ...noteControls(doc)]) {
    const f = type.fields.find((x) => x.name === c.field);
    if (f && !f.computed && controlFits(c.kind, f)) out.add(f.name);
  }
  return out;
}
