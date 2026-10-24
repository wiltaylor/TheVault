// Adding notes from a query view (`allow_new`, docs/vault-format.md, "Query views"): the type a
// new note takes and the fields it starts with, read from the view's query so the note it makes
// is one the view shows. Each view adds what its own layout says, such as a kanban column's term
// or the day a calendar cell stands for.

import { type FieldDef, type Json, type Note, type TypeDef, api } from "../api";
import { fromTypes } from "./narrow";

/** A query condition as `POST /api/query/parse` answers it in the builder model. */
interface Operand {
  kind: "str" | "int" | "float" | "bool" | "this" | "today" | "now";
  value?: Json;
}
interface Condition {
  field: string;
  op: "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "under" | "has" | "is_true" | "is_false";
  value: Operand | null;
}
type Item = { condition: Condition } | { group: Group };
interface Group {
  join: "all" | "any";
  items: Item[];
}
/** The parts of the builder model a new note is read from. */
export interface QueryModel {
  from: string[];
  filter: Group | null;
}

/** What a view's new notes are: their type and the fields every one starts with. */
export interface NewNoteSpec {
  type: string;
  fields: Record<string, Json>;
}

/** `YYYY-MM-DD` of `d` in local time. */
export function localDay(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** `d` as a datetime value: local time with its UTC offset, to the second. */
export function localDatetime(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  const off = -d.getTimezoneOffset();
  const sign = off >= 0 ? "+" : "-";
  const abs = Math.abs(off);
  return `${localDay(d)}T${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}${sign}${p(Math.floor(abs / 60))}:${p(abs % 60)}`;
}

/**
 * The value date field `f` takes for a note added at `at`: the day for a `date`, the moment for a
 * `datetime` (9 am on the day for a whole day), and a range over the day, or the hour from the
 * moment, for a `date_range`.
 */
export function dateValue(f: FieldDef, at: Date, wholeDay: boolean): Json {
  const moment = new Date(at);
  if (wholeDay) moment.setHours(9, 0, 0, 0);
  const hourLater = new Date(moment.getTime() + 60 * 60 * 1000);
  if (f.kind === "date") return localDay(at);
  if (f.kind === "datetime") return localDatetime(moment);
  if (f.of === "datetime" || (f.of === "date_or_datetime" && !wholeDay)) {
    return { start: localDatetime(moment), end: localDatetime(hourLater) };
  }
  return { start: localDay(at), end: localDay(at) };
}

/** A field's value for operand `o`, or undefined when the operand gives no fixed value. */
function operandValue(o: Operand | null, thisId: string | undefined): Json | undefined {
  if (!o) return undefined;
  switch (o.kind) {
    case "str":
    case "int":
    case "float":
    case "bool":
      return o.value;
    case "this":
      return thisId;
    case "today": {
      const d = new Date();
      d.setDate(d.getDate() + (typeof o.value === "number" ? o.value : 0));
      return localDay(d);
    }
    default:
      return undefined;
  }
}

/** `value` as field `f` holds it: a list for a field that holds several. */
function asFieldValue(f: FieldDef, value: Json): Json {
  const many = f.multiple || f.kind === "list";
  return many && !Array.isArray(value) ? [value] : value;
}

/**
 * The fields a new note of type `t` starts with so the query's `where` holds for it: every
 * condition joined by `&&` that names one of the type's own fields and fixes its value, as
 * `field == value`, `field == this`, `under(field, "term")`, `has(field, value)`, `field` and
 * `!field`. A condition under `||`, a comparison and a dotted path fix nothing and are left out.
 */
export function prefill(model: QueryModel, t: TypeDef, thisId: string | undefined): Record<string, Json> {
  const out: Record<string, Json> = {};
  const walk = (g: Group | null) => {
    if (!g || g.join !== "all") return;
    for (const item of g.items) {
      if ("group" in item) {
        walk(item.group);
        continue;
      }
      const c = item.condition;
      const f = t.fields.find((x) => x.name === c.field);
      if (!f || c.field === "title" || f.computed) continue;
      let value: Json | undefined;
      if (c.op === "is_true") value = true;
      else if (c.op === "is_false") value = false;
      else if (c.op === "eq" || c.op === "under" || c.op === "has") value = operandValue(c.value, thisId);
      if (value !== undefined) out[f.name] = asFieldValue(f, value);
    }
  };
  walk(model.filter);
  return out;
}

/**
 * What new notes of a view with query `query` are, or null when it reads no single type, such as
 * `from *` or `from a, b`. A query the builder cannot show still names its type; its fields then
 * start empty.
 */
export async function newNoteSpec(
  query: string,
  thisId: string | undefined,
  typeOf: (name: string) => TypeDef | undefined,
): Promise<NewNoteSpec | null> {
  const types = fromTypes(query);
  if (!types || types.length !== 1) return null;
  const t = typeOf(types[0]!);
  if (!t) return null;
  try {
    const parsed = (await api.parseQuery(query)) as { model?: QueryModel | null };
    return { type: t.name, fields: parsed.model ? prefill(parsed.model, t, thisId) : {} };
  } catch {
    return { type: t.name, fields: {} };
  }
}

/**
 * Create a note from `spec` titled `title`, with `extra` set over the spec's fields; an `extra`
 * of undefined leaves that field out, as a kanban board's "no term" column does.
 */
export function createFromView(spec: NewNoteSpec, title: string, extra: Record<string, Json | undefined> = {}): Promise<Note> {
  const fields: Record<string, Json> = { ...spec.fields };
  for (const [k, v] of Object.entries(extra)) {
    if (v === undefined) delete fields[k];
    else fields[k] = v;
  }
  return api.createNote(spec.type, { fields: { ...fields, title }, blocks: [] });
}
