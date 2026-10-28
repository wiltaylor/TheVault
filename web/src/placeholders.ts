// Placeholder links, `[[+type|title]]`: a link to a note that does not exist yet
// (docs/vault-format.md, "Inline markup"). Clicking one creates the note and rewrites the
// placeholder as a mention of it, in the note that holds it, through that note's normal save.
//
// Where the placeholder is rewritten:
// - In an editor, the editor replaces the node it was clicked on; the note's autosave saves it.
// - Elsewhere (a read-only card, a canvas card, the new-note form a type with required fields
//   opens), it is rewritten in the note that holds it: through that note's page when one is open
//   in this client, so its unsaved edits and the rewrite go out as one save, else through the
//   API as a versioned write over the version just read.

import { ApiError, type Block, type NoteDoc, type TypeDef, api } from "./api";
import { knowTitle } from "./editor/titles";
import { MARKUP_ITEMS, replacePlaceholder } from "./editor/markup";
import { cellsOf } from "./blocks/table";
import { missingRequired } from "./notes";
import { typeDef } from "./session";
import { followLink } from "./workspace/events";

// ---------------------------------------------------------------------------------------------
// Types offered for a new placeholder

const RECENT_KEY = "thevault.placeholderTypes";
const RECENT_MAX = 5;

/** The types most recently given to a placeholder, newest first. */
export function recentTypes(): string[] {
  try {
    const raw: unknown = JSON.parse(globalThis.localStorage?.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(raw) ? raw.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

/** Remember `name` as the newest type given to a placeholder. */
export function typeUsed(name: string) {
  const next = [name, ...recentTypes().filter((n) => n !== name)].slice(0, RECENT_MAX);
  try {
    globalThis.localStorage?.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    // Storage full or blocked: the order is a convenience only.
  }
}

/** How many types the picker offers before "Choose type…". */
export const SUGGESTED_TYPES = 3;
/** How many types a `type:` filter lists at most. */
export const FILTERED_TYPES = 8;

/** What the mention picker's text asks for: a title, and a type filter when it holds `type:`. */
export interface PickerQuery {
  /** The text with any `type:` word removed: what notes are searched for, and the new title. */
  title: string;
  /** The text after `type:`, or null when there is no `type:` word. */
  typeFilter: string | null;
}

/** Split the picker's text into the title and a `type:` filter, which may stand anywhere. */
export function readPickerQuery(text: string): PickerQuery {
  const m = /(^|\s)type:(\S*)/i.exec(text);
  if (!m) return { title: text.trim(), typeFilter: null };
  const title = (text.slice(0, m.index) + text.slice(m.index + m[0].length)).replace(/\s+/g, " ").trim();
  return { title, typeFilter: m[2]!.toLowerCase() };
}

/**
 * The types to offer for a new placeholder. Without a filter: the type of the note being edited,
 * then the recently used types, then the rest by label, at most [`SUGGESTED_TYPES`]. With a
 * filter: the types whose name or label starts with it, then those that contain it, at most
 * [`FILTERED_TYPES`].
 */
export function typeOptions(all: readonly TypeDef[], current: string | undefined, recent: readonly string[], filter: string | null): TypeDef[] {
  const byLabel = [...all].sort((a, b) => a.label.localeCompare(b.label));
  if (filter !== null) {
    const f = filter.toLowerCase();
    const starts = byLabel.filter((t) => t.name.startsWith(f) || t.label.toLowerCase().startsWith(f));
    const contains = byLabel.filter((t) => !starts.includes(t) && (t.name.includes(f) || t.label.toLowerCase().includes(f)));
    return [...starts, ...contains].slice(0, FILTERED_TYPES);
  }
  const order = [current, ...recent].filter((n): n is string => !!n);
  const first = order.map((n) => all.find((t) => t.name === n)).filter((t): t is TypeDef => !!t);
  const rest = byLabel.filter((t) => !first.includes(t));
  return [...new Set([...first, ...rest])].slice(0, SUGGESTED_TYPES);
}

// ---------------------------------------------------------------------------------------------
// Rewriting a placeholder in the note that holds it

/** `doc` with its first placeholder for `noteType` titled `title` rewritten as a mention of `id`, or null. */
export function replacePlaceholderInDoc(doc: NoteDoc, noteType: string, title: string, id: string): NoteDoc | null {
  let done = false;
  const walk = (blocks: Block[]): Block[] =>
    blocks.map((b) => {
      if (done) return b;
      const item = MARKUP_ITEMS[b.kind];
      const text = item ? b.fields?.[item] : undefined;
      if (item && typeof text === "string") {
        const next = replacePlaceholder(text, noteType, title, id);
        if (next !== null) {
          done = true;
          return { ...b, fields: { ...b.fields, [item]: next } };
        }
      }
      // A table's cells, in a row.
      if (b.kind === "row" && Array.isArray(b.fields?.cells)) {
        const cells = cellsOf(b);
        for (let c = 0; c < cells.length && !done; c++) {
          const next = replacePlaceholder(cells[c]!, noteType, title, id);
          if (next !== null) {
            done = true;
            cells[c] = next;
          }
        }
        if (done) return { ...b, fields: { ...b.fields, cells } };
      }
      if (!b.blocks?.length) return b;
      const inner = walk(b.blocks);
      return done ? { ...b, blocks: inner } : b;
    });
  const blocks = walk(doc.blocks);
  return done ? { ...doc, blocks } : null;
}

type LiveReplace = (noteType: string, title: string, id: string) => boolean;

/** The note pages open in this client, by note id, each able to rewrite a placeholder in its note. */
const live = new Map<string, Set<LiveReplace>>();

/** A note page offers to rewrite placeholders in note `id`. Returns the cleanup. */
export function offerPlaceholders(id: string, replace: LiveReplace): () => void {
  let set = live.get(id);
  if (!set) live.set(id, (set = new Set()));
  set.add(replace);
  return () => {
    set.delete(replace);
    if (set.size === 0 && live.get(id) === set) live.delete(id);
  };
}

/**
 * Rewrite the first placeholder for `noteType` titled `title` in note `host` as a mention of
 * `id`: through the note's open page if there is one, else as a versioned write. A write that
 * meets a newer version reads the note again and retries once. Returns whether it found one.
 */
export async function resolvePlaceholder(host: string, noteType: string, title: string, id: string): Promise<boolean> {
  for (const replace of live.get(host) ?? []) if (replace(noteType, title, id)) return true;
  for (let attempt = 0; ; attempt++) {
    const note = await api.note(host);
    const next = replacePlaceholderInDoc(note.note, noteType, title, id);
    if (!next) return false;
    try {
      await api.updateNote(host, next, note.version);
      return true;
    } catch (e) {
      if (attempt === 0 && e instanceof ApiError && e.isConflict) continue;
      throw e;
    }
  }
}

// ---------------------------------------------------------------------------------------------
// Clicking a placeholder

/** The new-note form for a placeholder: the type's page, the title filled in, and where it came from. */
export function placeholderFormUrl(noteType: string, title: string, host: string | undefined): string {
  const params = new URLSearchParams({ type: noteType, title });
  if (host) params.set("from", host);
  return `/notes/new?${params}`;
}

/**
 * Create a note of `noteType` titled `title`, or, when the type has a required field that has no
 * default, answer that the new-note form is needed.
 */
export async function createPlaceholderNote(noteType: string, title: string): Promise<{ id: string } | { form: true }> {
  const t = typeDef(noteType);
  if (!t) throw new Error(`There is no type ${noteType}.`);
  const doc: NoteDoc = { fields: { title }, blocks: [] };
  if (missingRequired(t, doc).length > 0) return { form: true };
  try {
    const saved = await api.createNote(noteType, doc);
    knowTitle(saved.id, title, saved.type);
    typeUsed(noteType);
    return { id: saved.id };
  } catch (e) {
    // The server knows rules this client does not check: the form shows them.
    if (e instanceof ApiError && e.status === 400) return { form: true };
    throw e;
  }
}

/** Clicks in progress and notes already created, by host, type and title. */
const inFlight = new Set<string>();
const created = new Map<string, string>();

export interface PlaceholderClick {
  /** The note whose stored text holds the placeholder, if known. */
  host: string | undefined;
  noteType: string;
  title: string;
  /** The element clicked: the new note opens in its pane. */
  from: Element;
  /** Open the new note in a background tab. */
  background: boolean;
  /** Rewrite the placeholder as a mention of `id`; resolves to whether it was found. */
  replace: (id: string) => boolean | Promise<boolean>;
}

/**
 * Follow a placeholder: create its note, rewrite it as a mention, and open the new note. A
 * second click while the first is under way does nothing, and a click on a placeholder whose
 * note this client already created opens that note rather than making another. A type whose
 * required fields need filling opens the new-note form instead, which rewrites the placeholder
 * in `host` once the note is created.
 */
export async function followPlaceholder(c: PlaceholderClick): Promise<void> {
  const key = `${c.host ?? ""}\u0000${c.noteType}\u0000${c.title}`;
  if (inFlight.has(key)) return;
  inFlight.add(key);
  try {
    let id = created.get(key);
    if (!id) {
      const out = await createPlaceholderNote(c.noteType, c.title);
      if ("form" in out) {
        followLink(placeholderFormUrl(c.noteType, c.title, c.host), c.from, c.background);
        return;
      }
      id = out.id;
      created.set(key, id);
    }
    await c.replace(id);
    followLink(`/notes/${id}`, c.from, c.background);
  } finally {
    inFlight.delete(key);
  }
}

/** The label of `noteType`, for "New Task" and the tooltip. */
export function typeLabel(noteType: string): string {
  return typeDef(noteType)?.label ?? noteType;
}
