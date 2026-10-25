// A type filter in a note picker's text: `person/sam` looks for "sam" among people only. The
// word before the first `/` names the type by its name, label or plural, case ignored: an exact
// match first, else the one type it starts. A word that names no type, or starts several, sets
// no filter, so the whole text is searched as typed and a title holding a `/` is still found.

import { type Page, type Row, type TypeDef, api } from "./api";
import { lit } from "./search/searchModel";

/** What a picker's text asks for, read against the types. */
export interface TypeFilter {
  /** The type the search is limited to, or null for every type. */
  type: TypeDef | null;
  /** The text to search for: what follows the `/` with a type, else the whole text, trimmed. */
  text: string;
  /** With no `/` yet: the types the text is the start of, to offer as filters. */
  suggestions: TypeDef[];
  /** A word before the `/` that names no one type: why there is no filter. Null otherwise. */
  problem: string | null;
}

/** How many types a picker offers as filters while the type is being typed. */
export const TYPE_SUGGESTIONS = 5;

type Named = Pick<TypeDef, "name" | "label" | "plural">;

const names = (t: Named) => [t.name, t.label, t.plural].filter(Boolean).map((x) => x.toLowerCase());
const byLabel = <T extends Named>(a: T, b: T) => a.label.localeCompare(b.label);

/** The types whose name, label or plural starts with `word`, already lower case, by label. */
function startedBy<T extends Named>(all: readonly T[], word: string): T[] {
  return all.filter((t) => names(t).some((n) => n.startsWith(word))).sort(byLabel);
}

/** The type `word` names, or the types it could be when it names none or several. */
export function matchType<T extends Named>(all: readonly T[], word: string): { type: T } | { candidates: T[] } {
  const w = word.trim().toLowerCase();
  const exact = all.filter((t) => names(t).includes(w));
  if (exact.length === 1) return { type: exact[0]! };
  if (exact.length > 1) return { candidates: exact.sort(byLabel) };
  const starts = startedBy(all, w);
  return starts.length === 1 ? { type: starts[0]! } : { candidates: starts };
}

/** Read a picker's text: a type filter before the first `/`, or the text whole. */
export function readTypeFilter(raw: string, all: readonly TypeDef[]): TypeFilter {
  const slash = raw.indexOf("/");
  const word = (slash < 0 ? raw : raw.slice(0, slash)).trim();
  if (slash < 0) {
    const suggestions = word ? startedBy(all, word.toLowerCase()).slice(0, TYPE_SUGGESTIONS) : [];
    return { type: null, text: raw.trim(), suggestions, problem: null };
  }
  const whole = { type: null, text: raw.trim(), suggestions: [] };
  if (!word) return { ...whole, problem: null };
  const m = matchType(all, word);
  if ("type" in m) return { type: m.type, text: raw.slice(slash + 1).trim(), suggestions: [], problem: null };
  if (m.candidates.length === 0) return { ...whole, problem: `No type matches “${word}”` };
  const shown = m.candidates.slice(0, 3).map((t) => t.label);
  const more = m.candidates.length > shown.length ? ` and ${m.candidates.length - shown.length} more` : "";
  return { ...whole, problem: `“${word}” matches ${shown.join(", ")}${more}: type more` };
}

/** The text that sets `type` as the filter, ready for the search after it. */
export const filterText = (type: Named) => `${type.name}/`;

/**
 * Up to `limit` notes for a picker: of `type`, when there is one, matching `text`, best match
 * first, or with no text the type's most recently changed; of any type, the notes `text`
 * matches, and none for no text. The server does the narrowing, so a page is never short.
 */
export async function searchNotes(type: string | null, text: string, limit: number): Promise<Row[]> {
  const t = text.trim();
  let page: Page;
  if (type === null) page = t ? await api.search(t, null, limit) : { rows: [], next: null, total: 0 };
  else page = await api.query(t ? `from ${type} search ${lit(t)} select title` : `from ${type} sort updated desc select title`, null, limit);
  return page.rows;
}
