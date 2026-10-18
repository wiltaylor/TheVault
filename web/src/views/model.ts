// A `query_view` content block (docs/vault-format.md, "Query views"; D8, D33) read into the shape
// the views render from, and the edits the block's settings make to it. Edits work on the block
// itself, so items this client does not edit yet, such as a canvas view's `place` blocks, are kept.

import type { Block, Json, Template, TypeDef } from "../api";
import type { Box } from "../canvas/canvas";
import { placesOf } from "./canvasView";

/** Every view kind the format allows, in the order the settings offer them. */
export const VIEW_KINDS = ["table", "cards", "list", "details", "kanban", "calendar", "timeline", "map", "canvas"];

/** The items each view kind uses besides `kind` and `title`, as `schema` checks them. */
const KIND_ITEMS: Record<string, string[]> = {
  table: ["columns"],
  cards: ["template"],
  details: ["template"],
  list: [],
  kanban: ["template", "group"],
  calendar: ["date", "mode"],
  timeline: ["date"],
  map: ["template", "location"],
  canvas: ["template"],
};

/** The calendar's `mode` values, the default first. */
export const CALENDAR_MODES = ["month", "week", "day"];

/** View names and template names are WCL block labels. */
export const IDENT = /^[A-Za-z_][A-Za-z0-9_]*$/;

export interface ViewDef {
  name: string;
  kind: string;
  /** The tab label: the view's `title`, else its name. */
  title: string;
  template: string | null;
  columns: string[];
  /** `map`: the location field pins sit at. */
  location: string | null;
  /** `kanban`: the single-term taxonomy field whose terms are the columns. */
  group: string | null;
  /** `calendar`, `timeline`: the date, datetime or date range field. */
  date: string | null;
  /** `calendar`: `month`, `week` or `day`. */
  mode: string;
  /** `canvas`: the saved card positions, by note id. */
  places: Map<string, Box>;
}

export interface QueryViewDef {
  title: string | null;
  query: string;
  views: ViewDef[];
  /** The view shown first: `default_view` when it names a view, else the first view. */
  first: ViewDef | undefined;
}

function str(fields: Record<string, Json> | undefined, item: string): string | null {
  const v = fields?.[item];
  return typeof v === "string" ? v : null;
}

export function viewBlocks(block: Block): Block[] {
  return (block.blocks ?? []).filter((b) => b.kind === "view");
}

export function readView(view: Block): ViewDef {
  const name = view.labels?.[0] ?? "";
  const columns = view.fields?.columns;
  return {
    name,
    kind: str(view.fields, "kind") ?? "table",
    title: str(view.fields, "title") ?? name,
    template: str(view.fields, "template"),
    columns: Array.isArray(columns) ? columns.filter((c): c is string => typeof c === "string") : ["title"],
    location: str(view.fields, "location"),
    group: str(view.fields, "group"),
    date: str(view.fields, "date"),
    mode: str(view.fields, "mode") ?? "month",
    places: placesOf(view),
  };
}

export function readQueryView(block: Block): QueryViewDef {
  const views = viewBlocks(block).map(readView);
  const dv = str(block.fields, "default_view");
  return {
    title: str(block.fields, "title"),
    query: str(block.fields, "query") ?? "",
    views,
    first: views.find((v) => v.name === dv) ?? views[0],
  };
}

/** A new query view block: every note, in one table view. */
export function newQueryView(): Block {
  return {
    kind: "query_view",
    fields: { query: "from * sort title asc" },
    blocks: [{ kind: "view", labels: ["table"], fields: { kind: "table" } }],
  };
}

/** `block` with item `name` set, or removed when `value` is undefined. */
export function withItem(block: Block, name: string, value: Json | undefined): Block {
  const fields = { ...block.fields };
  if (value === undefined) delete fields[name];
  else fields[name] = value;
  return { ...block, fields };
}

/** `block` with its `i`th view replaced by `edit` of it. Other blocks keep their places. */
function mapView(block: Block, i: number, edit: (view: Block) => Block): Block {
  let n = -1;
  return {
    ...block,
    blocks: (block.blocks ?? []).map((b) => (b.kind === "view" && ++n === i ? edit(b) : b)),
  };
}

export function setViewItem(block: Block, i: number, name: string, value: Json | undefined): Block {
  return mapView(block, i, (v) => withItem(v, name, value));
}

/** Rename view `i`, keeping `default_view` on it when it was the default. */
export function renameView(block: Block, i: number, name: string): Block {
  const old = viewBlocks(block)[i]?.labels?.[0];
  let next = mapView(block, i, (v) => ({ ...v, labels: [name] }));
  if (old !== undefined && str(block.fields, "default_view") === old) next = withItem(next, "default_view", name);
  return next;
}

/** Change view `i`'s kind, dropping the items the new kind does not use. */
export function setViewKind(block: Block, i: number, kind: string): Block {
  const keep = new Set(["kind", "title", ...(KIND_ITEMS[kind] ?? [])]);
  return mapView(block, i, (v) => {
    const fields: Record<string, Json> = {};
    for (const [k, value] of Object.entries(v.fields ?? {})) if (keep.has(k)) fields[k] = value;
    fields.kind = kind;
    // A `place` only means something to a canvas view.
    const blocks = kind === "canvas" ? v.blocks : undefined;
    return blocks?.length ? { ...v, fields, blocks } : { kind: v.kind, labels: v.labels, fields };
  });
}

/** Whether view kind `kind` uses item `item`. */
export function kindUses(kind: string, item: string): boolean {
  return KIND_ITEMS[kind]?.includes(item) ?? false;
}

/** Add a table view with a name no other view has. */
export function addView(block: Block): Block {
  const names = new Set(viewBlocks(block).map((v) => v.labels?.[0]));
  let n = viewBlocks(block).length + 1;
  while (names.has(`view${n}`)) n++;
  const view: Block = { kind: "view", labels: [`view${n}`], fields: { kind: "table" } };
  return { ...block, blocks: [...(block.blocks ?? []), view] };
}

/** Remove view `i`, and `default_view` with it when it named that view. */
export function removeView(block: Block, i: number): Block {
  const gone = viewBlocks(block)[i]?.labels?.[0];
  let n = -1;
  let next: Block = { ...block, blocks: (block.blocks ?? []).filter((b) => !(b.kind === "view" && ++n === i)) };
  if (gone !== undefined && str(block.fields, "default_view") === gone) next = withItem(next, "default_view", undefined);
  return next;
}

/** Show view `name` first. The first view is the default already, so naming it is left out. */
export function setDefaultView(block: Block, name: string): Block {
  const first = viewBlocks(block)[0]?.labels?.[0];
  return withItem(block, "default_view", name === first ? undefined : name);
}

/**
 * The template a view renders each note with (D14): the one the view names, else `card` for card
 * views or `default` for details, else `default`. A card shows only the first of its template's
 * tabs that shows for the note, which `TemplateBody` picks once it has the note.
 */
export function viewTemplate(type: TypeDef | undefined, name: string | null, use: "card" | "default"): Template | undefined {
  if (!type) return undefined;
  const find = (n: string) => type.templates.find((t) => t.name === n);
  return (name ? find(name) : undefined) ?? find(use) ?? find("default") ?? type.templates[0];
}
