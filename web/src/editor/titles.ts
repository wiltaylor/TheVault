// Note titles for mentions. A mention without a label shows its target's current title, read
// from the API, so renaming a note never rewrites the notes that mention it (D41).

import { createStore } from "solid-js/store";
import { ApiError, api } from "../api";

/** `undefined` while loading, `null` when the note does not exist or could not be read. */
const [titles, setTitles] = createStore<Record<string, string | null>>({});
/** Each loaded note's type, for the icon beside its title in tabs. */
const [types, setTypes] = createStore<Record<string, string>>({});
/** Whether each loaded note is archived, for the muted style of links to it. */
const [archived, setArchived] = createStore<Record<string, boolean>>({});
/** Notes whose title is known or loading. */
const requested = new Set<string>();
/** Notes read in full, or being read, so their archived state is known too. */
const loaded = new Set<string>();

/** Read note `id` and keep its title, type and archived state. */
function load(id: string) {
  requested.add(id);
  loaded.add(id);
  void api.note(id).then(
    (n) => {
      const t = n.note.fields.title;
      setTitles(id, typeof t === "string" ? t : null);
      setTypes(id, n.type);
      setArchived(id, n.note.fields.archived === true);
    },
    (e) => {
      setTitles(id, null);
      if (!(e instanceof ApiError)) console.warn(`could not load the title of ${id}`, e);
    },
  );
}

/** The current title of note `id`, loading it on first use. Reactive. */
export function noteTitle(id: string): string | null | undefined {
  if (!requested.has(id)) load(id);
  return titles[id];
}

/** The type of note `id`, once its title has loaded. Reactive. */
export function noteType(id: string): string | undefined {
  noteTitle(id);
  return types[id];
}

/**
 * Whether note `id` is archived, once it has loaded; `undefined` until then. A note whose title
 * came from a listing is read once to find out. Reactive.
 */
export function noteArchived(id: string): boolean | undefined {
  if (archived[id] === undefined && !loaded.has(id)) load(id);
  return archived[id];
}

/** Record whether a note is archived, after archiving it or reading it. */
export function knowArchived(id: string, value: boolean) {
  setArchived(id, value);
}

/** Record a title this client already knows, after a save or a search. */
export function knowTitle(id: string, title: string, type?: string) {
  requested.add(id);
  setTitles(id, title);
  if (type) setTypes(id, type);
}

/** Text for a mention: its label, else its target's title, else a placeholder. */
export function mentionText(id: string, label: string | null): string {
  if (label) return label;
  const t = noteTitle(id);
  if (t === undefined) return "…";
  return t ?? "Missing note";
}
