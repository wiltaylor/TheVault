// The type editor's model of a type file (D3, D6, D14), read from schema inspection and written
// back as WCL in the order docs/vault-format.md gives.

import type { Block, Json } from "../api";
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

/** Kinds that may hold several values. */
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
  multiple: boolean;
  default?: Json;
  options: string[];
  taxonomy: string;
  target: string;
  relation: (typeof RELATIONS)[number];
  inverse: string;
  on_delete: (typeof ON_DELETE)[number];
  items: (typeof SCALARS)[number];
  values: (typeof SCALARS)[number];
  of: "date" | "datetime";
  map: "geo" | "image";
  image: string;
}

export type TemplateItemModel = { slot: string; label: string | null } | { block: Block };

export interface TabModel {
  title: string;
  items: TemplateItemModel[];
}

export interface TemplateModel {
  name: string;
  label: string | null;
  tabs: TabModel[];
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
    multiple: false,
    options: [],
    taxonomy: "",
    target: "",
    relation: "one_way",
    inverse: "",
    on_delete: "unlink",
    items: "string",
    values: "string",
    of: "date",
    map: "geo",
    image: "",
  };
}

const str = (v: Json | undefined): string => (typeof v === "string" ? v : "");

function fieldFromSchema(f: Record<string, Json>): FieldModel {
  const name = str(f.name);
  const m = newField(name, str(f.kind) as FieldKind);
  m.original = name;
  m.label = str(f.label) === name ? "" : str(f.label);
  m.description = str(f.description);
  m.required = f.required === true;
  m.computed = f.computed === true;
  m.multiple = f.multiple === true;
  if (f.default !== undefined && f.default !== null) m.default = f.default;
  if (Array.isArray(f.options)) m.options = f.options.map(String);
  m.taxonomy = str(f.taxonomy);
  m.target = str(f.target);
  if (f.relation) m.relation = f.relation as FieldModel["relation"];
  m.inverse = str(f.inverse);
  if (f.on_delete) m.on_delete = f.on_delete as FieldModel["on_delete"];
  if (f.items) m.items = f.items as FieldModel["items"];
  if (f.values) m.values = f.values as FieldModel["values"];
  if (f.of) m.of = f.of as FieldModel["of"];
  m.image = str(f.image);
  m.map = f.map === "image" || m.image ? "image" : "geo";
  return m;
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
    if (f.kind === "select" && f.options.length === 0) out.push(`${who}: a select field needs options.`);
    if (f.kind === "taxonomy" && !f.taxonomy) out.push(`${who}: choose a taxonomy.`);
    if (f.kind === "reference") {
      if (!f.target) out.push(`${who}: choose a target type.`);
      if (f.relation === "two_way" && !f.inverse) out.push(`${who}: a two-way reference needs an inverse field.`);
      if (f.relation === "peer" && !f.multiple) out.push(`${who}: a peer reference holds several notes.`);
      if (f.relation !== "one_way" && f.target === "*") out.push(`${who}: only a one-way reference can target any type.`);
    }
    if (f.kind === "location" && f.map === "image" && !f.image) out.push(`${who}: choose the map image.`);
  }
  m.handlers.forEach((h, i) => {
    const who = `Handler ${i + 1} (${h.event})`;
    if (!h.script) out.push(`${who}: choose a script.`);
    if (!h.event.startsWith("after_") && (h.allow_domains.length || h.secrets.length)) {
      out.push(`${who}: only after_* handlers may reach hosts or read secrets.`);
    }
  });
  return out;
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

function fieldWcl(f: FieldModel): string {
  const i = "  ";
  let out = `field ${f.name} {\n`;
  out += wclField("kind", f.kind, i);
  if (f.label && f.label !== f.name) out += wclField("label", f.label, i);
  if (f.description) out += wclField("description", f.description, i);
  if (f.required) out += wclField("required", true, i);
  if (DEFAULT_KINDS.has(f.kind) && hasValue(f.default)) {
    const float = f.kind === "float" || (f.kind === "list" && f.items === "float");
    out += wclField("default", f.default, i, float);
  }
  if (f.computed) out += wclField("computed", true, i);
  if (f.multiple && MULTIPLE_KINDS.has(f.kind)) out += wclField("multiple", true, i);
  switch (f.kind) {
    case "date_range":
      out += wclField("of", f.of, i);
      break;
    case "select":
      out += wclField("options", f.options, i);
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
    case "list":
      out += wclField("items", f.items, i);
      break;
    case "dict":
      out += wclField("values", f.values, i);
      break;
    case "location":
      out += wclField("map", f.map, i);
      if (f.map === "image") out += wclField("image", f.image, i);
      break;
  }
  return `${out}}\n`;
}

/** One `template` block. */
export function templateWcl(t: TemplateModel): string {
  const parts: string[] = [];
  if (t.label) parts.push(wclField("label", t.label, "  "));
  for (const tab of t.tabs) {
    const items = tab.items.map((item) => {
      if ("block" in item) return wclBlock(item.block, "    ");
      const head = `    slot ${wclString(item.slot)}`;
      return item.label ? `${head} {\n${wclField("label", item.label, "      ")}    }\n` : `${head}\n`;
    });
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
  return m.templates.map((t) => ({
    ...t,
    tabs: t.tabs.map((tab) => ({
      ...tab,
      items: tab.items.flatMap((item): TemplateItemModel[] => {
        if (!("slot" in item)) return [{ block: followRefs(item.block, rename) }];
        if (item.slot === "title") return [item];
        const name = byOriginal.get(item.slot) ?? (known.has(item.slot) ? item.slot : undefined);
        return name === undefined || !known.has(name) ? [] : [{ ...item, slot: name }];
      }),
    })),
  }));
}

/** `block` with the `{{name}}` references in its text items, at any depth, renamed. */
function followRefs(block: Block, rename: (name: string) => string | null): Block {
  const text = block.fields?.text;
  const fields =
    typeof text === "string" && block.fields ? { ...block.fields, text: renameFieldRefs(text, rename) } : block.fields;
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
  const blocks = [...m.fields.map(fieldWcl), ...followFields(m).map(templateWcl), ...m.handlers.map(handlerWcl)];
  return [head, ...blocks].join("\n");
}

/** Every field a template does not place yet, in type order, with `title` first. */
export function unplaced(t: TemplateModel, fields: { name: string }[]): string[] {
  const placed = new Set(t.tabs.flatMap((tab) => tab.items.flatMap((i) => ("slot" in i ? [i.slot] : []))));
  return ["title", ...fields.map((f) => f.name)].filter((n) => !placed.has(n));
}
