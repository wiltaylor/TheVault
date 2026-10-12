// The visual builder's model of a query, as `query::Model` in crates/query/src/model.rs serialises
// it (D8). The server converts between text and model; this file only edits a model in place.

export type Join = "all" | "any";

export type Op = "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "under" | "has" | "is_true" | "is_false";

export type Operand =
  | { kind: "str"; value: string }
  | { kind: "int"; value: number }
  | { kind: "float"; value: number }
  | { kind: "bool"; value: boolean }
  | { kind: "this" }
  | { kind: "today"; value: number }
  | { kind: "now" };

export interface Condition {
  field: string;
  op: Op;
  /** Null exactly when `op` is `is_true` or `is_false`. */
  value: Operand | null;
}

export type Item = { condition: Condition } | { group: Group };

export interface Group {
  join: Join;
  items: Item[];
}

export type Order = "default" | "asc" | "desc";

export interface Sort {
  field: string;
  order: Order;
}

export interface QueryModel {
  /** Type names, or empty for `from *`. */
  from: string[];
  filter: Group | null;
  search: string | null;
  sort: Sort[];
  group_by: string | null;
  select: string[];
  limit: number | null;
}

/** The operators the builder offers, with the label each shows. */
export const OPS: [Op, string][] = [
  ["eq", "is"],
  ["ne", "is not"],
  ["lt", "<"],
  ["le", "≤"],
  ["gt", ">"],
  ["ge", "≥"],
  ["under", "is under"],
  ["has", "has"],
  ["is_true", "is set"],
  ["is_false", "is not set"],
];

/** The kinds of value the builder offers. `int` and `float` are one kind there: a number. */
export type ValueKind = "str" | "number" | "bool" | "this" | "today" | "now";

export const VALUE_KINDS: [ValueKind, string][] = [
  ["str", "text"],
  ["number", "number"],
  ["bool", "yes or no"],
  ["this", "this note"],
  ["today", "today"],
  ["now", "now"],
];

const takesNoValue = (op: Op) => op === "is_true" || op === "is_false";

export function newCondition(): Condition {
  return { field: "", op: "eq", value: { kind: "str", value: "" } };
}

export function valueKind(v: Operand): ValueKind {
  return v.kind === "int" || v.kind === "float" ? "number" : v.kind;
}

/** A fresh operand of `kind`, keeping what it can of `prev`. */
export function operandOfKind(kind: ValueKind, prev: Operand | null): Operand {
  switch (kind) {
    case "str":
      return { kind: "str", value: prev && "value" in prev ? String(prev.value) : "" };
    case "number": {
      const n = prev && "value" in prev ? Number(prev.value) : 0;
      return { kind: "int", value: Number.isFinite(n) ? Math.trunc(n) : 0 };
    }
    case "bool":
      return { kind: "bool", value: true };
    case "today":
      return { kind: "today", value: 0 };
    case "this":
      return { kind: "this" };
    case "now":
      return { kind: "now" };
  }
}

/**
 * The number typed in a value box, or null when it is not one yet. A number written with a
 * decimal point or an exponent is a float, so `2.0` stays `2.0` in the text.
 */
export function numberOperand(text: string): Operand | null {
  const t = text.trim();
  if (t === "") return null;
  const n = Number(t);
  if (!Number.isFinite(n)) return null;
  if (/[.eE]/.test(t)) return { kind: "float", value: n };
  return Number.isSafeInteger(n) ? { kind: "int", value: n } : null;
}

/** `c` with operator `op`, gaining or dropping its value as the operator needs. */
export function withOp(c: Condition, op: Op): Condition {
  if (takesNoValue(op)) return { ...c, op, value: null };
  return { ...c, op, value: c.value ?? { kind: "str", value: "" } };
}

/** `g` with its `i`th item replaced, or removed when `next` is null. */
export function withGroupItem(g: Group, i: number, next: Item | null): Group {
  const items = g.items.slice();
  if (next === null) items.splice(i, 1);
  else items[i] = next;
  return { ...g, items };
}

/** A comma-separated list of names, trimmed, without empties. */
export function nameList(text: string): string[] {
  return text
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}
