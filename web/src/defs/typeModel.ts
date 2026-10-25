// The type editor's model of a type file (D3, D6, D14), read from schema inspection and written
// back as WCL in the order docs/vault-format.md gives.

import type { Block, FieldDef, Json, SchemaIssue } from "../api";
import { slotOfBlock, slotsOf } from "../blocks/columns";
import { renameFieldRefs } from "../editor/markup";
import { wclBlock, wclField, wclString } from "./wcl";

export const FIELD_KINDS = [
  "string",
  "int",
  "float",
  "bool",
  "date",
  "datetime",
  "date_range",
  "url",
  "select",
  "taxonomy",
  "reference",
  "list",
  "dict",
  "location",
  "content",
  "asset",
  "ink",
  "canvas",
] as const;

export type FieldKind = (typeof FIELD_KINDS)[number];

export const SCALARS = ["string", "int", "float", "bool", "date", "datetime", "url"] as const;
/** What a list item, a dict key or a dict's other entries may be: every kind but the block kinds (types T1). */
export const VALUE_KINDS = [
  "string",
  "int",
  "float",
  "bool",
  "date",
  "datetime",
  "date_range",
  "url",
  "select",
  "taxonomy",
  "reference",
  "list",
  "dict",
  "location",
  "asset",
] as const;
export const RELATIONS = ["one_way", "two_way", "peer"] as const;
export const ON_DELETE = ["unlink", "restrict", "cascade"] as const;
export const PALETTE = ["gray", "brown", "orange", "yellow", "green", "blue", "purple", "pink", "red"] as const;
export const EVENTS = [
  "before_create",
  "before_update",
  "before_delete",
  "after_create",
  "after_update",
  "after_delete",
] as const;

/** What a date range's ends may be, as the type editor offers them. */
export const RANGE_OF = [
  { value: "date", label: "Dates" },
  { value: "datetime", label: "Dates and times" },
  { value: "date_or_datetime", label: "Dates or dates and times" },
] as const;

/** Kinds a list of which is that kind holding several values (types T2). */
export const MULTIPLE_KINDS = new Set<string>(["select", "taxonomy", "reference", "asset"]);
/** Kinds that may carry a `default`. */
export const DEFAULT_KINDS = new Set<string>([
  "string",
  "int",
  "float",
  "bool",
  "date",
  "datetime",
  "url",
  "select",
  "taxonomy",
  "list",
]);
/** Kinds written as a block in the note. */
export const COMPOUND_KINDS = new Set<string>(["content", "dict", "ink", "canvas"]);

/** What a list holds, by kind, or null for a definition that is not a list. */
export function itemKind(f: Pick<FieldModel, "kind" | "item">): FieldKind | null {
  return f.kind === "list" ? (f.item?.kind ?? "string") : null;
}

/**
 * Where a definition sits (types T2): a type's field, the item of a field-level list of selects,
 * terms, references or assets (which is that field holding several), the item of any other list,
 * a dict's key, or what a dict's other entries are.
 */
export type Place = "field" | "fieldItem" | "item" | "key" | "values";

/** Where the item of a list at `place` sits, when it is of kind `kind`. */
export function itemPlace(place: Place, kind: string): Place {
  return place === "field" && MULTIPLE_KINDS.has(kind) ? "fieldItem" : "item";
}

/** The relations a reference at `place` may have: two-way and peer only at the top (types T2). */
export function relationsAt(place: Place): readonly (typeof RELATIONS)[number][] {
  if (place === "fieldItem") return RELATIONS;
  if (place === "field") return ["one_way", "two_way"];
  return ["one_way"];
}

const KEY_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** Whether `s` may name a dict key: letters, digits and _, not starting with a digit. */
export function isKeyName(s: string): boolean {
  return KEY_NAME.test(s);
}

const NAME = /^[a-z][a-z0-9_]*$/;
const RESERVED = new Set(["title", "created", "updated"]);

export function isName(s: string): boolean {
  return NAME.test(s) && s.length <= 64;
}

export interface FieldModel {
  /** Stable while editing, so a renamed field keeps its row. */
  key: number;
  /** The field's name in the stored type, or null for a field added in this edit. */
  original: string | null;
  name: string;
  kind: FieldKind;
  label: string;
  description: string;
  required: boolean;
  computed: boolean;
  hidden: boolean;
  default?: Json;
  options: string[];
  /** One label per option, index-aligned; "" shows the value. Empty when no option has one. */
  option_labels: string[];
  taxonomy: string;
  target: string;
  relation: (typeof RELATIONS)[number];
  inverse: string;
  on_delete: (typeof ON_DELETE)[number];
  /** A list: what it holds, a definition of its own. A list of selects, terms, references or assets is that kind holding several. */
  item: FieldModel | null;
  /** A dict: its declared keys, each a definition named by its key. */
  keys: FieldModel[];
  /** A dict: what its other entries are, none (only declared keys), any kind, or one definition. */
  values: null | "any" | FieldModel;
  of: (typeof RANGE_OF)[number]["value"];
  map: "geo" | "image";
  image: string;
  /** A list of strings: the list-of-strings field holding one label per value, or "". */
  labels: string;
  /** An asset field: which assets it takes. */
  accept: "any" | "image";
  /** Attributes this version does not model, by name, as WCL source text: written back as read. */
  extra: Record<string, string>;
}

export type TemplateItemModel = { slot: string; label: string | null; when?: string | null; extra?: Record<string, string> } | { block: Block };

export interface TabModel {
  title: string;
  /** The tab's `when` condition, or none to show it for every note. */
  when?: string | null;
  items: TemplateItemModel[];
  /** Attributes this version does not model, as WCL source text. */
  extra?: Record<string, string>;
}

export interface TemplateModel {
  name: string;
  label: string | null;
  tabs: TabModel[];
  /** Attributes this version does not model, as WCL source text. */
  extra?: Record<string, string>;
}

export interface HandlerModel {
  event: string;
  script: string;
  function: string | null;
  allow_domains: string[];
  secrets: string[];
}

export interface TypeModel {
  label: string;
  plural: string;
  description: string;
  icon: string;
  colour: string;
  fields: FieldModel[];
  templates: TemplateModel[];
  handlers: HandlerModel[];
  /** The type's list page settings: its `list_page` block (lists L1). */
  listPage: ListPageModel;
  /** The type's own attributes this version does not model, as WCL source text. */
  extra: Record<string, string>;
}

/**
 * A type's `list_page` block, each item `""` when the block leaves it out, so the default holds.
 * The file has the block only when one item is set.
 */
export interface ListPageModel {
  view: "" | "cards" | "list" | "details";
  /** A field of the type, `title`, `created` or `updated`. */
  sortField: string;
  sortDir: "asc" | "desc";
  /** A whole number from 10 to 500, as typed. */
  pageSize: string;
}

/** The built-in note fields a list page may sort by. */
export const LIST_SORT_BUILT_INS = ["title", "created", "updated"] as const;

export function emptyListPage(): ListPageModel {
  return { view: "", sortField: "", sortDir: "desc", pageSize: "" };
}

function listPageFromSchema(lp: SchemaType["list_page"]): ListPageModel {
  const m = emptyListPage();
  if (!lp) return m;
  if (lp.view === "cards" || lp.view === "list" || lp.view === "details") m.view = lp.view;
  const [field, dir] = (lp.sort ?? "").split(/\s+/);
  if (field) {
    m.sortField = field;
    m.sortDir = dir === "desc" ? "desc" : "asc";
  }
  if (typeof lp.page_size === "number") m.pageSize = String(lp.page_size);
  return m;
}

/** The field a list page sorts by, following a rename; `null` when the type no longer has it. */
function listSortField(m: TypeModel): string | null {
  const s = m.listPage.sortField;
  if (!s || (LIST_SORT_BUILT_INS as readonly string[]).includes(s)) return s;
  if (m.fields.some((f) => f.name === s)) return s;
  return m.fields.find((f) => f.original === s)?.name ?? null;
}

function listPageWcl(m: TypeModel): string | null {
  const lp = m.listPage;
  const field = listSortField(m);
  const i = "  ";
  let out = "";
  if (lp.view) out += wclField("view", lp.view, i);
  if (field) out += wclField("sort", `${field} ${lp.sortDir}`, i);
  if (lp.pageSize.trim()) out += wclField("page_size", Number(lp.pageSize), i);
  return out ? `list_page {\n${out}}\n` : null;
}

/** A type as `GET /api/schema` describes it. */
export interface SchemaType {
  name: string;
  label: string;
  plural: string;
  description: string | null;
  icon: string | null;
  colour: string | null;
  fields: Record<string, Json>[];
  templates: TemplateModel[];
  handlers?: HandlerModel[];
  /** The `list_page` block, each item only when the block sets it. */
  list_page?: { view?: string; sort?: string; page_size?: number };
  /** What this version ignored in the type file, such as an attribute only a newer release knows. */
  warnings?: SchemaIssue[];
  /** What it ignored of the type's own attributes, as WCL source text, kept to write back. */
  extra?: Record<string, string>;
}

let nextKey = 1;

export function newField(name = "", kind: FieldKind = "string"): FieldModel {
  return {
    key: nextKey++,
    original: null,
    name,
    kind,
    label: "",
    description: "",
    required: false,
    computed: false,
    hidden: false,
    options: [],
    option_labels: [],
    taxonomy: "",
    target: "",
    relation: "one_way",
    inverse: "",
    on_delete: "unlink",
    item: null,
    keys: [],
    values: null,
    of: "date",
    map: "geo",
    image: "",
    labels: "",
    accept: "any",
    extra: {},
  };
}

/**
 * Definition `f` turned into kind `kind`: its default goes, a list gets an item (text) and a dict
 * takes entries of text until it is given keys.
 */
export function changeKind(f: FieldModel, kind: FieldKind) {
  f.kind = kind;
  f.default = undefined;
  if (kind === "list" && !f.item) f.item = newField(f.name);
  if (kind === "dict" && f.keys.length === 0 && f.values === null) f.values = newField(f.name);
}

/** Whether field `f` may carry a `default`: a list only of scalars, selects or terms. */
export function takesDefault(f: FieldModel): boolean {
  if (f.kind === "list") return ([...SCALARS, "select", "taxonomy"] as string[]).includes(itemKind(f)!);
  return DEFAULT_KINDS.has(f.kind);
}

const str = (v: Json | undefined): string => (typeof v === "string" ? v : "");

const isObject = (v: Json | undefined): v is Record<string, Json> => !!v && typeof v === "object" && !Array.isArray(v);

function fieldFromSchema(f: Record<string, Json>): FieldModel {
  const name = str(f.name);
  const kind = str(f.kind) as FieldKind;
  const m = newField(name, kind);
  m.original = name;
  m.label = str(f.label) === name ? "" : str(f.label);
  m.description = str(f.description);
  m.required = f.required === true;
  m.computed = f.computed === true;
  m.hidden = f.hidden === true;
  if (f.default !== undefined && f.default !== null) m.default = f.default;
  if (isObject(f.extra)) {
    for (const [k, v] of Object.entries(f.extra)) if (typeof v === "string") m.extra[k] = v;
  }
  kindFromSchema(m, f);
  // A select, taxonomy, reference or asset holding several is a list of that kind (types T2),
  // whether the file declares it as a list or with the old `multiple = true`.
  if (MULTIPLE_KINDS.has(kind) && (f.listed === true || f.multiple === true)) {
    const item = newField(name, kind);
    kindFromSchema(item, f);
    m.kind = "list";
    m.item = item;
  }
  return m;
}

/** The attributes of `f`'s kind, and its item, keys and other entries, into `m`. */
function kindFromSchema(m: FieldModel, f: Record<string, Json>) {
  if (Array.isArray(f.options)) m.options = f.options.map(String);
  if (Array.isArray(f.option_labels)) m.option_labels = f.option_labels.map(String);
  m.taxonomy = str(f.taxonomy);
  m.target = str(f.target);
  if (f.relation) m.relation = f.relation as FieldModel["relation"];
  m.inverse = str(f.inverse);
  if (f.on_delete) m.on_delete = f.on_delete as FieldModel["on_delete"];
  if (f.of) m.of = f.of as FieldModel["of"];
  m.image = str(f.image);
  m.map = f.map === "image" || m.image ? "image" : "geo";
  m.labels = str(f.labels);
  if (f.accept === "image") m.accept = "image";
  if (m.kind === "list") {
    if (isObject(f.item)) m.item = fieldFromSchema(f.item);
    else m.item = newField(m.name, (str(f.items) || "string") as FieldKind);
    m.item.original = null;
  }
  if (m.kind === "dict") {
    m.keys = Array.isArray(f.keys) ? f.keys.filter(isObject).map(fieldFromSchema) : [];
    const v = f.values;
    m.values = v === "any" ? "any" : isObject(v) ? fieldFromSchema(v) : typeof v === "string" && v ? newField(m.name, v as FieldKind) : null;
    if (m.values && m.values !== "any") m.values.original = null;
  }
}

export function typeFromSchema(t: SchemaType): TypeModel {
  return {
    label: t.label,
    plural: t.plural === t.label ? "" : t.plural,
    description: t.description ?? "",
    icon: t.icon ?? "",
    colour: t.colour ?? "",
    fields: t.fields.map(fieldFromSchema),
    templates: structuredClone(t.templates),
    handlers: structuredClone(t.handlers ?? []),
    listPage: listPageFromSchema(t.list_page),
    extra: { ...(t.extra ?? {}) },
  };
}

/** A new type: no fields, and a default template showing the title. */
export function emptyType(): TypeModel {
  return {
    label: "",
    plural: "",
    description: "",
    icon: "",
    colour: "",
    fields: [],
    templates: [{ name: "default", label: null, tabs: [{ title: "Main", items: [{ slot: "title", label: null }] }] }],
    handlers: [],
    listPage: emptyListPage(),
    extra: {},
  };
}

/** Old name to new name, for every stored field whose name changed. */
export function renames(m: TypeModel): Record<string, string> {
  const out: Record<string, string> = {};
  for (const f of m.fields) if (f.original !== null && f.original !== f.name) out[f.original] = f.name;
  return out;
}

/** What is wrong with the model before the server sees it. Empty when nothing is. */
export function problems(m: TypeModel): string[] {
  const out: string[] = [];
  if (!m.label.trim()) out.push("The type needs a label.");
  const seen = new Set<string>();
  for (const f of m.fields) {
    const who = f.name || "A field";
    if (!isName(f.name)) out.push(`${JSON.stringify(f.name)} is not a field name: use a-z, 0-9 and _.`);
    else if (RESERVED.has(f.name)) out.push(`${f.name} is reserved.`);
    else if (seen.has(f.name)) out.push(`${f.name} is used twice.`);
    seen.add(f.name);
    if (f.required && f.computed) out.push(`${who}: a computed field cannot be required.`);
    if (f.required && f.hidden) out.push(`${who}: a hidden field cannot be required.`);
    defProblems(m, f, who, "field", out);
  }
  m.handlers.forEach((h, i) => {
    const who = `Handler ${i + 1} (${h.event})`;
    if (!h.script) out.push(`${who}: choose a script.`);
    if (!h.event.startsWith("after_") && (h.allow_domains.length || h.secrets.length)) {
      out.push(`${who}: only after_* handlers may reach hosts or read secrets.`);
    }
  });
  if (m.listPage.sortField && listSortField(m) === null) {
    out.push(`List page: it sorts by ${m.listPage.sortField}, which the type no longer has.`);
  }
  const size = m.listPage.pageSize.trim();
  if (size && !(/^\d+$/.test(size) && Number(size) >= 10 && Number(size) <= 500)) {
    out.push("List page: the page size must be a whole number from 10 to 500.");
  }
  return out;
}

/** What is wrong with definition `f`, sitting at `place`, and with its item, keys and other entries. */
function defProblems(m: TypeModel, f: FieldModel, who: string, place: Place, out: string[]) {
  if (f.kind === "select" && f.options.length === 0) out.push(`${who}: a select field needs options.`);
  if (f.kind === "select" && new Set(f.options).size !== f.options.length) out.push(`${who}: an option is listed twice.`);
  if (f.kind === "taxonomy" && !f.taxonomy) out.push(`${who}: choose a taxonomy.`);
  if (f.kind === "reference") {
    if (!f.target) out.push(`${who}: choose a target type.`);
    if (f.relation === "two_way" && !f.inverse) out.push(`${who}: a two-way reference needs an inverse field.`);
    if (f.relation === "peer" && place !== "fieldItem") out.push(`${who}: a peer reference holds several notes: make it a list of references.`);
    if (f.relation !== "one_way" && f.target === "*") out.push(`${who}: only a one-way reference can target any type.`);
    if (f.relation !== "one_way" && place !== "field" && place !== "fieldItem") {
      out.push(`${who}: inside a dictionary or a nested list a reference is one-way.`);
    }
  }
  if (f.kind === "location" && f.map === "image" && !f.image) out.push(`${who}: choose the map image.`);
  if (f.kind === "list") {
    const item = f.item;
    if (!item) {
      out.push(`${who}: choose what the list holds.`);
      return;
    }
    if (!(VALUE_KINDS as readonly string[]).includes(item.kind)) out.push(`${who}: a list cannot hold ${item.kind} items.`);
    if (place === "field" && item.kind === "string" && f.labels) {
      const l = m.fields.find((g) => g.name === f.labels || g.original === f.labels);
      if (f.labels === f.name) out.push(`${who}: labels cannot name the field itself.`);
      else if (!l) out.push(`${who}: labels names no field ${JSON.stringify(f.labels)}.`);
      else if (itemKind(l) !== "string") out.push(`${who}: labels must name a list of strings.`);
    }
    defProblems(m, item, place === "field" ? who : `${who} item`, itemPlace(place, item.kind), out);
  }
  if (f.kind === "dict") {
    if (f.keys.length === 0 && f.values === null) out.push(`${who}: a dictionary needs keys, other keys, or both.`);
    const seen = new Set<string>();
    for (const k of f.keys) {
      if (!isKeyName(k.name)) {
        out.push(`${who}: ${JSON.stringify(k.name)} is not a key name: use letters, digits and _, not starting with a digit.`);
      } else if (seen.has(k.name)) out.push(`${who}: key ${k.name} is used twice.`);
      seen.add(k.name);
      if (!(VALUE_KINDS as readonly string[]).includes(k.kind)) out.push(`${who}.${k.name}: a dictionary key cannot be ${k.kind}.`);
      defProblems(m, k, `${who}.${k.name || "key"}`, "key", out);
    }
    if (f.values && f.values !== "any") {
      if (!(VALUE_KINDS as readonly string[]).includes(f.values.kind)) out.push(`${who}: other keys cannot be ${f.values.kind}.`);
      defProblems(m, f.values, `${who} (other keys)`, "values", out);
    }
  }
}

/** New required fields without a default, which a type that has notes cannot take (D6). */
export function needsDefault(m: TypeModel): string[] {
  return m.fields
    .filter((f) => f.required && f.original === null && !COMPOUND_KINDS.has(f.kind) && !hasValue(f.default))
    .map((f) => f.name);
}

export function hasValue(v: Json | undefined): boolean {
  return v !== undefined && v !== null && v !== "" && !(Array.isArray(v) && v.length === 0);
}

function fieldWcl(f: FieldModel, renamed: Record<string, string> = {}): string {
  return `field ${f.name} {\n${defWcl(f, "  ", "field", renamed)}}\n`;
}

/** Whether a list's item is written as `items = "<kind>"` with its attributes beside it, not as an `item` block. */
function shorthandItem(item: FieldModel): boolean {
  if (MULTIPLE_KINDS.has(item.kind)) return true;
  return (
    item.kind !== "list" &&
    item.kind !== "dict" &&
    !(item.label && item.label !== item.name) &&
    !item.description &&
    !hasValue(item.default) &&
    Object.keys(item.extra).length === 0
  );
}

/** Whether a dict's other entries are written as `values = "<kind>"`: a kind without attributes. */
function shorthandValues(v: FieldModel): boolean {
  return (
    (SCALARS as readonly string[]).includes(v.kind) &&
    !(v.label && v.label !== v.name) &&
    !v.description &&
    !hasValue(v.default) &&
    Object.keys(v.extra).length === 0
  );
}

/** A definition's body at `place`, each line indented by `i`: its attributes, then its blocks. */
function defWcl(f: FieldModel, i: string, place: Place, renamed: Record<string, string>): string {
  let out = wclField("kind", f.kind, i);
  if (f.label && f.label !== f.name) out += wclField("label", f.label, i);
  if (f.description) out += wclField("description", f.description, i);
  if (f.required && (place === "field" || place === "key")) out += wclField("required", true, i);
  if (DEFAULT_KINDS.has(f.kind) && hasValue(f.default)) {
    const float = f.kind === "float" || itemKind(f) === "float";
    out += wclField("default", f.default, i, float);
  }
  if (place === "field") {
    if (f.computed) out += wclField("computed", true, i);
    if (f.hidden) out += wclField("hidden", true, i);
  }
  let blocks = "";
  const inner = `${i}  `;
  switch (f.kind) {
    case "list": {
      const item = f.item ?? newField(f.name);
      if (shorthandItem(item)) {
        out += wclField("items", item.kind, i);
        out += kindWcl(item, i);
        // A renamed label field is followed.
        if (item.kind === "string" && f.labels) out += wclField("labels", renamed[f.labels] ?? f.labels, i);
      } else {
        blocks += `${i}item {\n${defWcl(item, inner, itemPlace(place, item.kind), renamed)}${i}}\n`;
      }
      break;
    }
    case "dict":
      for (const k of f.keys) blocks += `${i}key ${k.name} {\n${defWcl(k, inner, "key", renamed)}${i}}\n`;
      if (f.values === "any") out += wclField("values", "any", i);
      else if (f.values && shorthandValues(f.values)) out += wclField("values", f.values.kind, i);
      else if (f.values) blocks += `${i}values {\n${defWcl(f.values, inner, "values", renamed)}${i}}\n`;
      break;
    default:
      out += kindWcl(f, i);
  }
  return `${out}${extraWcl(f.extra, out, i)}${blocks}`;
}

/** The attributes of `f`'s kind: a select's options, a reference's target and relation, and so on. */
function kindWcl(f: FieldModel, i: string): string {
  let out = "";
  switch (f.kind) {
    case "date_range":
      out += wclField("of", f.of, i);
      break;
    case "select":
      out += wclField("options", f.options, i);
      // Labels are written only when an option has one, one per option.
      if (f.option_labels.some(Boolean)) {
        out += wclField("option_labels", f.options.map((_, n) => f.option_labels[n] ?? ""), i);
      }
      break;
    case "taxonomy":
      out += wclField("taxonomy", f.taxonomy, i);
      break;
    case "reference":
      out += wclField("target", f.target, i);
      out += wclField("relation", f.relation, i);
      if (f.relation === "two_way") out += wclField("inverse", f.inverse, i);
      out += wclField("on_delete", f.on_delete, i);
      break;
    case "asset":
      if (f.accept === "image") out += wclField("accept", "image", i);
      break;
    case "location":
      out += wclField("map", f.map, i);
      if (f.map === "image") out += wclField("image", f.image, i);
      break;
  }
  return out;
}

/** Field `f` as a note page's editors take it, as `GET /api/types` describes a field. */
export function fieldDefOf(f: FieldModel): FieldDef {
  if (f.kind === "list" && f.item && MULTIPLE_KINDS.has(f.item.kind)) {
    // A list of selects, terms, references or assets: that kind holding several.
    return { ...fieldDefOf(f.item), ...ownDef(f), kind: f.item.kind, multiple: true, listed: true, items: f.item.kind };
  }
  const d: FieldDef = {
    ...ownDef(f),
    kind: f.kind,
    multiple: false,
    compound: COMPOUND_KINDS.has(f.kind),
    options: f.options,
    option_labels: f.option_labels,
    taxonomy: f.taxonomy,
    target: f.target,
    relation: f.relation,
    inverse: f.inverse || null,
    of: f.of,
    image: f.map === "image" ? f.image : null,
    accept: f.accept,
  };
  if (f.kind === "list") {
    const item = f.item ?? newField(f.name);
    d.items = item.kind;
    d.item = fieldDefOf(item);
    d.labels = f.labels || null;
  }
  if (f.kind === "dict") {
    d.keys = f.keys.map(fieldDefOf);
    d.values = f.values === "any" ? "any" : f.values ? fieldDefOf(f.values) : null;
  }
  return d;
}

function ownDef(f: FieldModel) {
  return {
    name: f.name,
    label: f.label || f.name,
    description: f.description || null,
    required: f.required,
    computed: f.computed,
    hidden: f.hidden,
  };
}

/**
 * The attributes this version does not model, written back as they were read, after `written`:
 * one the editor now writes itself, such as `labels` on a field that became a list of strings,
 * is left out so it is not written twice.
 */
function extraWcl(extra: Record<string, string> | undefined, written: string, indent: string): string {
  return Object.entries(extra ?? {})
    .filter(([name]) => !new RegExp(`^${indent}${name} = `, "m").test(written))
    .map(([name, text]) => `${indent}${name} = ${text}\n`)
    .join("");
}

/** One option per line, as `value` or `value = Label`. */
export function optionsText(f: Pick<FieldModel, "options" | "option_labels">): string {
  return f.options.map((o, n) => (f.option_labels[n] ? `${o} = ${f.option_labels[n]}` : o)).join("\n");
}

/** The options and their labels from [`optionsText`]'s lines; a line without ` = ` has no label. */
export function parseOptions(text: string): { options: string[]; option_labels: string[] } {
  const options: string[] = [];
  const option_labels: string[] = [];
  for (const line of text.split("\n")) {
    const at = line.indexOf(" = ");
    const value = (at >= 0 ? line.slice(0, at) : line).trim();
    if (!value) continue;
    options.push(value);
    option_labels.push(at >= 0 ? line.slice(at + 3).trim() : "");
  }
  return { options, option_labels: option_labels.some(Boolean) ? option_labels : [] };
}

/** One `template` block. */
export function templateWcl(t: TemplateModel): string {
  const parts: string[] = [];
  const head = t.label ? wclField("label", t.label, "  ") : "";
  const own = head + extraWcl(t.extra, head, "  ");
  if (own) parts.push(own);
  for (const tab of t.tabs) {
    const items = tab.items.map((item) => {
      if ("block" in item) return wclBlock(item.block, "    ");
      const head = `    slot ${wclString(item.slot)}`;
      const when = item.when?.trim();
      const label =
        (item.label ? wclField("label", item.label, "      ") : "") + (when ? wclField("when", when, "      ") : "");
      const body = label + extraWcl(item.extra, label, "      ");
      return body ? `${head} {\n${body}    }\n` : `${head}\n`;
    });
    const when = tab.when?.trim();
    const attrs = when ? wclField("when", when, "    ") : "";
    const tabOwn = attrs + extraWcl(tab.extra, attrs, "    ");
    if (tabOwn) items.unshift(tabOwn);
    parts.push(`  tab ${wclString(tab.title)}${items.length ? ` {\n${items.join("")}  }` : ""}\n`);
  }
  return `template ${t.name} {\n${parts.join("\n")}}\n`;
}

function handlerWcl(h: HandlerModel): string {
  const i = "  ";
  let out = `handler ${wclString(h.event)} {\n`;
  out += wclField("script", h.script, i);
  if (h.function) out += wclField("function", h.function, i);
  if (h.allow_domains.length) out += wclField("allow_domains", h.allow_domains, i);
  if (h.secrets.length) out += wclField("secrets", h.secrets, i);
  return `${out}}\n`;
}

/**
 * Templates with every slot following the fields: a renamed field's slots take its new name, and
 * a removed field's slots go.
 */
export function followFields(m: TypeModel): TemplateModel[] {
  const byOriginal = new Map(m.fields.filter((f) => f.original !== null).map((f) => [f.original!, f.name]));
  const known = new Set(m.fields.map((f) => f.name));
  // `{{name}}` in fixed blocks follows too; a removed field's reference is left as plain text.
  const rename = (name: string) => {
    if (name === "title") return name;
    const to = byOriginal.get(name) ?? (known.has(name) ? name : undefined);
    return to !== undefined && known.has(to) ? to : null;
  };
  // A slot's field after the change, or null when the field is gone.
  const slotName = (slot: string): string | null => {
    if (slot === "title") return slot;
    const name = byOriginal.get(slot) ?? (known.has(slot) ? slot : undefined);
    return name === undefined || !known.has(name) ? null : name;
  };
  return m.templates.map((t) => ({
    ...t,
    tabs: t.tabs.map((tab) => ({
      ...tab,
      items: tab.items.flatMap((item): TemplateItemModel[] => {
        if (!("slot" in item)) {
          const block = followControls(followSlots(followRefs(item.block, rename), slotName), slotName);
          return block ? [{ block }] : [];
        }
        const name = slotName(item.slot);
        return name === null ? [] : [{ ...item, slot: name }];
      }),
    })),
  }));
}

/** A `columns` block with the slots in its columns following their fields. Any other block as it is. */
function followSlots(block: Block, slotName: (slot: string) => string | null): Block {
  if (block.kind !== "columns") return block;
  return {
    ...block,
    blocks: (block.blocks ?? []).map((col) => ({
      ...col,
      blocks: (col.blocks ?? []).flatMap((b) => {
        const slot = slotOfBlock(b);
        if (slot === null) return [b];
        const name = slotName(slot);
        return name === null ? [] : [{ ...b, labels: [name] }];
      }),
    })),
  };
}

/**
 * `block` with every control, and every image from a field, in it, at any depth, bound to its
 * field's new name; one whose field is gone goes too, and so does `block` itself when it is one
 * (null).
 */
function followControls(block: Block, fieldName: (name: string) => string | null): Block | null {
  if (block.kind === "control" || (block.kind === "image" && typeof block.fields?.field === "string")) {
    const field = block.fields?.field;
    if (typeof field !== "string") return block;
    const to = fieldName(field);
    return to === null ? null : { ...block, fields: { ...block.fields, field: to } };
  }
  if (!block.blocks) return block;
  return { ...block, blocks: block.blocks.flatMap((b) => followControls(b, fieldName) ?? []) };
}

/** `block` with the `{{name}}` references in its text items (and an image's alt and caption), at any depth, renamed. */
function followRefs(block: Block, rename: (name: string) => string | null): Block {
  let fields = block.fields;
  for (const item of block.kind === "image" ? ["text", "alt", "caption"] : ["text"]) {
    const text = fields?.[item];
    if (typeof text === "string" && fields) fields = { ...fields, [item]: renameFieldRefs(text, rename) };
  }
  const blocks = block.blocks?.map((b) => followRefs(b, rename));
  const out: Block = { ...block };
  if (fields) out.fields = fields;
  if (blocks) out.blocks = blocks;
  return out;
}

/** The type file. Slots and `{{name}}` references follow renamed and removed fields. */
export function typeWcl(m: TypeModel): string {
  let head = wclField("label", m.label.trim());
  if (m.plural.trim()) head += wclField("plural", m.plural.trim());
  if (m.description.trim()) head += wclField("description", m.description.trim());
  if (m.icon.trim()) head += wclField("icon", m.icon.trim());
  if (m.colour) head += wclField("colour", m.colour);
  head += extraWcl(m.extra, head, "");
  const renamed = renames(m);
  const blocks = [...m.fields.map((f) => fieldWcl(f, renamed)), ...followFields(m).map(templateWcl), ...m.handlers.map(handlerWcl)];
  const listPage = listPageWcl(m);
  if (listPage) blocks.push(listPage);
  return [head, ...blocks].join("\n");
}

/**
 * Every field a template does not place yet, in type order, with `title` first. A field holding
 * another field's labels is edited with that field, so it is not offered.
 */
export function unplaced(t: TemplateModel, fields: { name: string; labels?: string | null }[]): string[] {
  const placed = new Set(t.tabs.flatMap((tab) => slotsOf(tab.items)));
  const labelFields = new Set(fields.map((f) => f.labels).filter(Boolean));
  return ["title", ...fields.map((f) => f.name)].filter((n) => !placed.has(n) && !labelFields.has(n));
}
