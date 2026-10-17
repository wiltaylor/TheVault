// How a note's JSON form maps onto its type's fields and templates (D14).

import type { Block, FieldDef, Json, NoteDoc, Template, TypeDef } from "./api";
import { slotsOf } from "./blocks/columns";
import { controlledFields } from "./controls/model";

/** The built-in title, treated as a string field wherever a template places it. */
export const TITLE_FIELD: FieldDef = {
  name: "title",
  label: "Title",
  description: null,
  kind: "string",
  required: true,
  computed: false,
  multiple: false,
  compound: false,
};

/** The field a slot names: `title` or one of the type's fields. */
export function slotField(type: TypeDef, name: string): FieldDef | undefined {
  return name === "title" ? TITLE_FIELD : type.fields.find((f) => f.name === name);
}

/** The note page's template: `default`, which every type has. */
export function pageTemplate(type: TypeDef): Template | undefined {
  return type.templates.find((t) => t.name === "default") ?? type.templates[0];
}

/** A compound field's block: the block of the field's kind labelled with the field's name. */
export function compoundBlock(doc: NoteDoc, f: FieldDef): Block | undefined {
  return doc.blocks.find((b) => b.kind === f.kind && b.labels?.[0] === f.name);
}

/** A field's value in `doc`, as comparable JSON. */
export function fieldValue(doc: NoteDoc, f: FieldDef): Json | undefined {
  if (f.compound) {
    const b = compoundBlock(doc, f);
    if (f.kind === "dict") return b?.fields;
    return b === undefined ? undefined : (b as unknown as Json);
  }
  return doc.fields[f.name];
}

/** `doc` with `edits` applied. An `undefined` edit removes the field. */
export function applyEdits(doc: NoteDoc, edits: Record<string, Json | undefined>): NoteDoc {
  const fields = { ...doc.fields };
  for (const [name, value] of Object.entries(edits)) {
    if (value === undefined) delete fields[name];
    else fields[name] = value;
  }
  return { fields, blocks: doc.blocks };
}

/**
 * `doc` with compound field `f` holding `blocks`: the field's block is replaced in place, added
 * when it was missing, or removed when `blocks` is empty.
 */
export function applyBlocks(doc: NoteDoc, f: FieldDef, blocks: Block[]): NoteDoc {
  const at = doc.blocks.findIndex((b) => b.kind === f.kind && b.labels?.[0] === f.name);
  const next = [...doc.blocks];
  if (blocks.length === 0) {
    if (at >= 0) next.splice(at, 1);
  } else {
    const old = next[at];
    if (old) next[at] = { ...old, blocks };
    else next.push({ kind: f.kind, labels: [f.name], blocks });
  }
  return { fields: doc.fields, blocks: next };
}

/** `doc` with compound field `f`'s block replaced by `block`, labelled with the field name. */
export function applyCompound(doc: NoteDoc, f: FieldDef, block: Block): NoteDoc {
  const labelled: Block = { ...block, kind: f.kind, labels: [f.name] };
  const at = doc.blocks.findIndex((b) => b.kind === f.kind && b.labels?.[0] === f.name);
  const next = [...doc.blocks];
  if (at >= 0) next[at] = labelled;
  else next.push(labelled);
  return { fields: doc.fields, blocks: next };
}

/** `doc` with dict field `f` holding `entries`, or without its block when `entries` is undefined. */
export function applyDict(doc: NoteDoc, f: FieldDef, entries: Json | undefined): NoteDoc {
  const at = doc.blocks.findIndex((b) => b.kind === "dict" && b.labels?.[0] === f.name);
  const next = [...doc.blocks];
  const record = entries && typeof entries === "object" && !Array.isArray(entries) ? entries : undefined;
  if (record === undefined || Object.keys(record).length === 0) {
    if (at >= 0) next.splice(at, 1);
  } else if (at >= 0) {
    next[at] = { ...next[at]!, fields: record };
  } else {
    next.push({ kind: "dict", labels: [f.name], fields: record });
  }
  return { fields: doc.fields, blocks: next };
}

/**
 * The fields to compare in a conflict: the title and the type's fields in type order, then any
 * other stored field. `created`, `updated` and `archived` are the server's, so they are left out.
 */
export function comparedFields(type: TypeDef | undefined, a: NoteDoc, b: NoteDoc): FieldDef[] {
  const known = [TITLE_FIELD, ...(type?.fields ?? [])];
  const names = new Set(known.map((f) => f.name));
  const extra = [...Object.keys(a.fields), ...Object.keys(b.fields)]
    .filter((n) => !names.has(n) && n !== "created" && n !== "updated" && n !== "archived")
    .filter((n, i, all) => all.indexOf(n) === i)
    .map((n) => ({ ...TITLE_FIELD, name: n, label: n, required: false }));
  return [...known, ...extra];
}

export function sameValue(a: Json | undefined, b: Json | undefined): boolean {
  return JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
}

/** The fields the server stamps: a merge takes them from the stored side. */
const STAMPED = new Set(["created", "updated", "archived"]);

/** A doc's blocks by a key of their kind, first label and place among blocks of that key. */
function keyedBlocks(doc: NoteDoc): Map<string, Block> {
  const out = new Map<string, Block>();
  const seen = new Map<string, number>();
  for (const b of doc.blocks) {
    const k = `${b.kind}\u0000${b.labels?.[0] ?? ""}`;
    const n = seen.get(k) ?? 0;
    seen.set(k, n + 1);
    out.set(`${k}\u0000${n}`, b);
  }
  return out;
}

/** One value of a three-way merge, `null` on a clash. `{ v: undefined }` is "absent". */
function pick<T>(base: T | undefined, mine: T | undefined, theirs: T | undefined, preferMine: boolean): { v: T | undefined } | null {
  const same = (a: T | undefined, b: T | undefined) => sameValue(a as Json | undefined, b as Json | undefined) && (a === undefined) === (b === undefined);
  if (same(mine, theirs)) return { v: mine };
  if (same(mine, base)) return { v: theirs };
  if (same(theirs, base) || preferMine) return { v: mine };
  return null;
}

/**
 * Three-way merge of a note, field by field and compound block by compound block: `mine` and
 * `theirs` both started from `base`. Each side keeps what it changed and the other did not; a
 * field both changed differently is a clash, and the merge is `null`. With `preferMine` a clash
 * keeps `mine`, which makes this a rebase of `mine`'s edits over `theirs`. The server's stamps
 * (`created`, `updated`, `archived`) come from `theirs`.
 */
export function mergeDocs(base: NoteDoc, mine: NoteDoc, theirs: NoteDoc, preferMine = false): NoteDoc | null {
  const fields: Record<string, Json> = {};
  const names = [...Object.keys(theirs.fields), ...Object.keys(mine.fields), ...Object.keys(base.fields)];
  for (const name of new Set(names)) {
    if (STAMPED.has(name)) {
      if (theirs.fields[name] !== undefined) fields[name] = theirs.fields[name];
      continue;
    }
    const got = pick(base.fields[name], mine.fields[name], theirs.fields[name], preferMine);
    if (!got) return null;
    if (got.v !== undefined) fields[name] = got.v;
  }
  const b = keyedBlocks(base);
  const m = keyedBlocks(mine);
  const t = keyedBlocks(theirs);
  const blocks: Block[] = [];
  // Theirs' order, then the blocks only mine has, in mine's order.
  for (const key of new Set([...t.keys(), ...m.keys(), ...b.keys()])) {
    const got = pick(b.get(key), m.get(key), t.get(key), preferMine);
    if (!got) return null;
    if (got.v !== undefined) blocks.push(got.v);
  }
  return { fields, blocks };
}

/** Whether two docs hold the same compound blocks, which editors with state of their own show. */
export function sameBlocks(a: NoteDoc, b: NoteDoc): boolean {
  return JSON.stringify(a.blocks) === JSON.stringify(b.blocks);
}

/** The fields the type's default template places with a slot, on any tab and in any column. */
export function slottedFields(type: TypeDef): Set<string> {
  const t = pageTemplate(type);
  return new Set((t?.tabs ?? []).flatMap((tab) => slotsOf(tab.items)));
}

/**
 * Whether the type's default template places field `name` with a slot or an editable control, so
 * an empty image from that field need not offer an upload of its own.
 */
export function templatePlaces(type: TypeDef, name: string): boolean {
  return slottedFields(type).has(name) || controlledFields(type).has(name);
}

/**
 * The type's fields the note page has no slot or editable control for, in type order: the
 * properties sidebar edits these. A control in the template or in the note's own content places
 * its field as a slot does; a read-only control on a computed field and a `{{name}}` reference
 * do not count as a place. A hidden field is left out.
 */
export function unplacedFields(type: TypeDef, doc?: NoteDoc): FieldDef[] {
  const placed = slottedFields(type);
  for (const name of controlledFields(type, doc)) placed.add(name);
  return type.fields.filter((f) => !placed.has(f.name) && !f.hidden);
}

/**
 * The value `{{name}}` shows for a note: the title, a field's value, or `null` when the type has
 * no field `name`. A compound field other than a dict shows nothing.
 */
export function refValue(type: TypeDef | undefined, doc: NoteDoc, name: string): Json | undefined | null {
  const f = name === "title" ? TITLE_FIELD : type?.fields.find((x) => x.name === name);
  if (!f) return null;
  if (f.compound && f.kind !== "dict") return undefined;
  return fieldValue(doc, f);
}

/** True when `v` is no value at all: missing, empty text or an empty list. */
export function isBlank(v: Json | undefined): boolean {
  return v === undefined || v === null || v === "" || (Array.isArray(v) && v.length === 0);
}

/**
 * The required fields a new note still lacks, by label: the title, then each required field
 * that is not computed and has no default for the server to fill in.
 */
export function missingRequired(type: TypeDef, doc: NoteDoc): string[] {
  const fields = [TITLE_FIELD, ...type.fields].filter((f) => f.required && !f.computed && f.default === undefined);
  return fields.filter((f) => isBlank(fieldValue(doc, f))).map((f) => f.label);
}
