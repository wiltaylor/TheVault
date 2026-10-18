// The model behind a type's note list (`/types/:type`): which view it shows, remembered per
// type in this browser; the columns its List view has; the query it runs; and its sort.

import type { FieldDef, Json, Row, Template, TypeDef } from "./api";
import { slotsOf } from "./blocks/columns";
import { labelFields } from "./notes";
import { lit } from "./search/searchModel";

/**
 * The most notes a type's list loads. A type with more shows only its search box until a
 * search narrows it, and a search shows at most this many results.
 */
export const MAX_LISTED = 200;

export type TypeListView = "cards" | "list";

export const TYPE_VIEWS_KEY = "thevault.typeList.views";

function stored(): Record<string, TypeListView> {
  try {
    const raw = JSON.parse(localStorage.getItem(TYPE_VIEWS_KEY) ?? "{}");
    return raw && typeof raw === "object" && !Array.isArray(raw) ? raw : {};
  } catch {
    return {};
  }
}

/** The view `type`'s list last showed in this browser; Cards when it has none. */
export function loadTypeView(type: string): TypeListView {
  return stored()[type] === "list" ? "list" : "cards";
}

export function saveTypeView(type: string, view: TypeListView) {
  const all = stored();
  if (view === "cards") delete all[type];
  else all[type] = view;
  try {
    localStorage.setItem(TYPE_VIEWS_KEY, JSON.stringify(all));
  } catch {
    // Private browsing: the choice lasts until the page closes.
  }
}

/**
 * The type's `card` template, or undefined when it has none. A card shows only the first of its
 * tabs that shows for the note (`TemplateBody`).
 */
export function cardTemplate(type: TypeDef): Template | undefined {
  return type.templates.find((x) => x.name === "card");
}

/** Fields a List view cell can show on one line. */
function scalar(f: FieldDef): boolean {
  return !f.compound && !["content", "location", "ink", "canvas", "dict"].includes(f.kind);
}

/** How many fields the List view shows when the type has no card template to pick them. */
export const FALLBACK_COLUMNS = 3;

/**
 * The List view's field columns, between Title and Updated: the fields the card template
 * places, else the type's first few scalar fields.
 */
export function listColumns(type: TypeDef): FieldDef[] {
  const card = cardTemplate(type);
  // A field holding another's labels shows within that field's column.
  const labels = labelFields(type);
  const byName = new Map(type.fields.filter((f) => !labels.has(f.name)).map((f) => [f.name, f]));
  if (card) {
    const placed = [...new Set(card.tabs.slice(0, 1).flatMap((tab) => slotsOf(tab.items)))]
      .map((s) => byName.get(s))
      .filter((f): f is FieldDef => !!f && scalar(f));
    if (placed.length) return placed;
  }
  return type.fields.filter((f) => scalar(f) && !labels.has(f.name)).slice(0, FALLBACK_COLUMNS);
}

/**
 * The query for a type's notes: by title, or by relevance when there is search text. As on the
 * Search page, the last word matches as a prefix until a space follows it. Archived notes are
 * left out unless `archived` is true.
 */
export function typeQuery(type: string, text: string, archived = false): string {
  const from = `from ${type}${archived ? " include archived" : ""}`;
  const t = text.trim();
  if (!t) return `${from} sort title asc`;
  return `${from} search ${lit(/\s$/.test(text) ? `${t} ` : t)}`;
}

export type SortDir = "asc" | "desc";

/** A sortable column: `title`, `updated`, or a field's name. */
export interface ListSort {
  key: string;
  dir: SortDir;
}

function cell(row: Row, key: string): Json | undefined {
  return key === "title" ? row.title : row.fields[key];
}

function isEmpty(v: Json | undefined): boolean {
  return v === undefined || v === null || v === "" || (Array.isArray(v) && v.length === 0);
}

function text(v: Json): string {
  if (Array.isArray(v)) return v.map(String).join(", ");
  return typeof v === "object" ? JSON.stringify(v) : String(v);
}

function compare(a: Json, b: Json): number {
  if (typeof a === "number" && typeof b === "number") return a - b;
  if (typeof a === "boolean" && typeof b === "boolean") return Number(a) - Number(b);
  return text(a).localeCompare(text(b), undefined, { numeric: true, sensitivity: "base" });
}

/** `rows` sorted by one column. Empty values sort last either way; ties keep their order. */
export function sortRows(rows: readonly Row[], sort: ListSort | null): Row[] {
  if (!sort) return rows.slice();
  const sign = sort.dir === "asc" ? 1 : -1;
  return rows
    .map((row, i) => ({ row, i, v: cell(row, sort.key) }))
    .sort((x, y) => {
      const ex = isEmpty(x.v);
      const ey = isEmpty(y.v);
      if (ex || ey) return ex === ey ? x.i - y.i : ex ? 1 : -1;
      return sign * compare(x.v!, y.v!) || x.i - y.i;
    })
    .map((x) => x.row);
}

/** The sort after clicking `key`'s header: ascending first, then descending, then off. */
export function nextSort(sort: ListSort | null, key: string): ListSort | null {
  if (sort?.key !== key) return { key, dir: "asc" };
  return sort.dir === "asc" ? { key, dir: "desc" } : null;
}

/** `n` notes, with thousands separated. */
export function noteCount(n: number): string {
  return n === 1 ? "1 note" : `${n.toLocaleString("en-US")} notes`;
}
