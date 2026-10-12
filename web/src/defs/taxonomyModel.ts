// The taxonomy editor's model of a taxonomy file (D7): a term tree that remembers where each
// stored term started, so renames and moves can rewrite the notes holding them.

import { wclField, wclString } from "./wcl";

export interface TermModel {
  key: number;
  /** The term's stored path, or null for a term added in this edit. */
  original: string | null;
  name: string;
  colour: string;
  description: string;
  children: TermModel[];
}

export interface TaxonomyModel {
  label: string;
  description: string;
  terms: TermModel[];
}

/** A taxonomy as `GET /api/taxonomies` describes it. */
export interface SchemaTaxonomy {
  name: string;
  label: string;
  description: string | null;
  terms: SchemaTerm[];
}

export interface SchemaTerm {
  name: string;
  path: string;
  colour: string | null;
  description: string | null;
  children: SchemaTerm[];
}

let nextKey = 1;

export function newTerm(name = ""): TermModel {
  return { key: nextKey++, original: null, name, colour: "", description: "", children: [] };
}

function termFromSchema(t: SchemaTerm): TermModel {
  return {
    key: nextKey++,
    original: t.path,
    name: t.name,
    colour: t.colour ?? "",
    description: t.description ?? "",
    children: t.children.map(termFromSchema),
  };
}

export function taxonomyFromSchema(t: SchemaTaxonomy): TaxonomyModel {
  return { label: t.label, description: t.description ?? "", terms: t.terms.map(termFromSchema) };
}

/** Every term with its path in the edited tree, parents before children. */
export function walk(terms: TermModel[], prefix = ""): { term: TermModel; path: string; depth: number }[] {
  return terms.flatMap((term) => {
    const path = prefix ? `${prefix}/${term.name}` : term.name;
    const depth = prefix ? prefix.split("/").length : 0;
    return [{ term, path, depth }, ...walk(term.children, path)];
  });
}

/** Old path to new path for every stored term that was renamed or moved. */
export function termRenames(m: TaxonomyModel): Record<string, string> {
  const out: Record<string, string> = {};
  for (const { term, path } of walk(m.terms)) {
    if (term.original !== null && term.original !== path) out[term.original] = path;
  }
  return out;
}

export function problems(m: TaxonomyModel): string[] {
  const out: string[] = [];
  if (!m.label.trim()) out.push("The taxonomy needs a label.");
  if (m.terms.length === 0) out.push("A taxonomy needs at least one term.");
  const check = (list: TermModel[], where: string) => {
    const seen = new Set<string>();
    for (const t of list) {
      if (!t.name.trim()) out.push(`A term ${where} has no name.`);
      else if (t.name.includes("/")) out.push(`${t.name}: a term name cannot contain "/".`);
      else if (seen.has(t.name)) out.push(`${t.name} appears twice ${where}.`);
      seen.add(t.name);
      check(t.children, `under ${t.name}`);
    }
  };
  check(m.terms, "at the top level");
  return out;
}

function termWcl(t: TermModel, indent: string): string {
  const inner = `${indent}  `;
  const fields = wclField("colour", t.colour || undefined, inner) + wclField("description", t.description || undefined, inner);
  const children = t.children.map((c) => termWcl(c, inner));
  const head = `${indent}term ${wclString(t.name)}`;
  if (!fields && children.length === 0) return `${head}\n`;
  return `${head} {\n${[fields, ...children].filter(Boolean).join("\n")}${indent}}\n`;
}

export function taxonomyWcl(m: TaxonomyModel): string {
  const head = wclField("label", m.label.trim()) + wclField("description", m.description.trim() || undefined);
  return [head, ...m.terms.map((t) => termWcl(t, ""))].join("\n");
}

/** The list holding `key` and its index there. */
export function locate(
  terms: TermModel[],
  key: number,
): { list: TermModel[]; index: number; parent: TermModel | null; parentList: TermModel[] | null } | null {
  const search = (list: TermModel[], parent: TermModel | null, parentList: TermModel[] | null): ReturnType<typeof locate> => {
    for (let i = 0; i < list.length; i++) {
      if (list[i]!.key === key) return { list, index: i, parent, parentList };
      const found = search(list[i]!.children, list[i]!, list);
      if (found) return found;
    }
    return null;
  };
  return search(terms, null, null);
}
