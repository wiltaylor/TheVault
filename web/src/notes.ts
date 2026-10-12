// How a note's JSON form maps onto its type's fields and templates (D14).

import type { Block, FieldDef, Json, NoteDoc, Template, TypeDef } from "./api";

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

/** Kinds the shell edits with a plain input. The rest wait for their editors. */
const EDITABLE_KINDS = new Set(["string", "int", "float", "bool", "date", "datetime", "url", "select"]);

export function isEditable(f: FieldDef): boolean {
  return !f.compound && !f.computed && !f.multiple && EDITABLE_KINDS.has(f.kind);
}

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

/**
 * The fields to compare in a conflict: the title and the type's fields in type order, then any
 * other stored field. `created` and `updated` are the server's, so they are left out.
 */
export function comparedFields(type: TypeDef | undefined, a: NoteDoc, b: NoteDoc): FieldDef[] {
  const known = [TITLE_FIELD, ...(type?.fields ?? [])];
  const names = new Set(known.map((f) => f.name));
  const extra = [...Object.keys(a.fields), ...Object.keys(b.fields)]
    .filter((n) => !names.has(n) && n !== "created" && n !== "updated")
    .filter((n, i, all) => all.indexOf(n) === i)
    .map((n) => ({ ...TITLE_FIELD, name: n, label: n, required: false }));
  return [...known, ...extra];
}

export function sameValue(a: Json | undefined, b: Json | undefined): boolean {
  return JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
}

/** Parse an input's text into the field's encoding. Empty text means no value. */
export function parseInput(f: FieldDef, raw: string): Json | undefined {
  if (raw === "") return undefined;
  if (f.kind === "int") {
    const n = Number(raw);
    return Number.isFinite(n) ? Math.trunc(n) : raw;
  }
  if (f.kind === "float") {
    const n = Number(raw);
    return Number.isFinite(n) ? n : raw;
  }
  return raw;
}
