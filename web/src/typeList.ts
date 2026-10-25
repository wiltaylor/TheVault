// The model behind a type's note list (`/types/:type`): which view and sort it shows, from the
// type's `list_page` settings or the user's choice remembered per type in this browser (lists L1,
// L3); the columns its List view has; the query it runs; and how its page status reads.

import type { FieldDef, Template, TypeDef } from "./api";
import { slotsOf } from "./blocks/columns";
import { lit } from "./search/searchModel";

/** The views the asset library remembers with `loadTypeView`. */
export type TypeListView = "cards" | "list";

/** The views a type's list page shows its notes in (lists L4). */
export type TypePageView = "cards" | "list" | "details";

export const TYPE_PAGE_VIEWS: readonly TypePageView[] = ["cards", "list", "details"];

export type SortDir = "asc" | "desc";

/** A sortable column: `title`, `updated`, `created`, or a field's name. */
export interface ListSort {
  key: string;
  dir: SortDir;
}

/** What a type's list page shows when neither the type nor the user says (lists L1). */
export const DEFAULT_VIEW: TypePageView = "cards";
export const DEFAULT_SORT: ListSort = { key: "updated", dir: "desc" };
export const DEFAULT_PAGE_SIZE = 50;

export const TYPE_VIEWS_KEY = "thevault.typeList.views";

function stored(): Record<string, TypeListView> {
  try {
    const raw = JSON.parse(localStorage.getItem(TYPE_VIEWS_KEY) ?? "{}");
    return raw && typeof raw === "object" && !Array.isArray(raw) ? raw : {};
  } catch {
    return {};
  }
}

/** The view the asset library (or a type's list, before lists L3) last showed; Cards when none. */
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

/** The type's `list_page` settings, with the defaults filled in (lists L1). */
export interface ListPageSettings {
  view: TypePageView;
  sort: ListSort;
  pageSize: number;
}

/** `"sent desc"` as a sort; `undefined` for anything else. A sort with no direction is ascending. */
export function parseSort(text: string | undefined): ListSort | undefined {
  const [key, dir = "asc", ...rest] = (text ?? "").trim().split(/\s+/);
  if (!key || rest.length || (dir !== "asc" && dir !== "desc")) return undefined;
  return { key, dir };
}

export function listPageSettings(type: TypeDef): ListPageSettings {
  const lp = type.list_page;
  const view = TYPE_PAGE_VIEWS.find((v) => v === lp?.view) ?? DEFAULT_VIEW;
  const size = lp?.page_size;
  return {
    view,
    sort: parseSort(lp?.sort) ?? DEFAULT_SORT,
    pageSize: typeof size === "number" && size >= 10 && size <= 500 ? size : DEFAULT_PAGE_SIZE,
  };
}

/** What the user chose on a type's list page, remembered in this browser (lists L3). */
export interface TypePrefs {
  view?: TypePageView;
  sort?: ListSort;
}

export const TYPE_PREFS_KEY = "thevault.typeList.prefs";

function storedPrefs(): Record<string, TypePrefs> {
  try {
    const raw = JSON.parse(localStorage.getItem(TYPE_PREFS_KEY) ?? "{}");
    return raw && typeof raw === "object" && !Array.isArray(raw) ? raw : {};
  } catch {
    return {};
  }
}

/**
 * The view and sort the user chose for `type`'s list page, each left out when they chose none. A
 * List view chosen before lists L3 still counts, unless the type has a `list_page` block, whose
 * settings then win until the user chooses again.
 */
export function loadTypePrefs(type: TypeDef): TypePrefs {
  const raw = storedPrefs()[type.name];
  const out: TypePrefs = {};
  if (raw && typeof raw === "object") {
    const view = TYPE_PAGE_VIEWS.find((v) => v === raw.view);
    if (view) out.view = view;
    const s = raw.sort;
    if (s && typeof s.key === "string" && (s.dir === "asc" || s.dir === "desc")) out.sort = { key: s.key, dir: s.dir };
    return out;
  }
  if (!type.list_page && stored()[type.name] === "list") out.view = "list";
  return out;
}

/** Remember `prefs` for `type`; an empty choice forgets it, so the type's settings apply. */
export function saveTypePrefs(type: string, prefs: TypePrefs) {
  const all = storedPrefs();
  if (!prefs.view && !prefs.sort) delete all[type];
  else all[type] = prefs;
  try {
    localStorage.setItem(TYPE_PREFS_KEY, JSON.stringify(all));
    // The older per-type view gives way to this record.
    const views = stored();
    if (type in views) {
      delete views[type];
      localStorage.setItem(TYPE_VIEWS_KEY, JSON.stringify(views));
    }
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
  const byName = new Map(type.fields.map((f) => [f.name, f]));
  if (card) {
    const placed = [...new Set(card.tabs.slice(0, 1).flatMap((tab) => slotsOf(tab.items)))]
      .map((s) => byName.get(s))
      .filter((f): f is FieldDef => !!f && scalar(f));
    if (placed.length) return placed;
  }
  return type.fields.filter(scalar).slice(0, FALLBACK_COLUMNS);
}

/**
 * The query for a type's notes, sorted by `sort` on the server so that paging stays in order
 * (lists L3), and narrowed by search text when there is any. As on the Search page, the last word
 * matches as a prefix until a space follows it. Archived notes are left out unless `archived` is
 * true.
 */
export function typeQuery(type: string, text: string, archived: boolean, sort: ListSort): string {
  const from = `from ${type}${archived ? " include archived" : ""}`;
  const t = text.trim();
  const search = t ? ` search ${lit(/\s$/.test(text) ? `${t} ` : t)}` : "";
  return `${from}${search} sort ${sort.key} ${sort.dir}`;
}

/**
 * The sort of a type's list page after clicking `key`'s header: the other direction when it
 * sorts by `key` already, else ascending.
 */
export function flipSort(sort: ListSort, key: string): ListSort {
  if (sort.key !== key) return { key, dir: "asc" };
  return { key, dir: sort.dir === "asc" ? "desc" : "asc" };
}

export function sameSort(a: ListSort, b: ListSort): boolean {
  return a.key === b.key && a.dir === b.dir;
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

const num = (n: number) => n.toLocaleString("en-US");

/**
 * The status line of a type's list page: the count when one page holds every note, else the
 * range page `index` shows, such as "51–100 of 684" (lists L2).
 */
export function pageStatus(index: number, pageSize: number, shown: number, total: number, searching: boolean): string {
  if (index === 0 && shown >= total) {
    if (!searching) return noteCount(total);
    return total === 1 ? "1 match" : `${num(total)} matches`;
  }
  const from = index * pageSize + 1;
  const range = shown === 0 ? `None of ${num(total)}` : `${num(from)}–${num(from + shown - 1)} of ${num(total)}`;
  return searching ? `${range} matches` : range;
}
