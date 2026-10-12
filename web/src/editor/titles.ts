// Note titles for mentions. A mention without a label shows its target's current title, read
// from the API, so renaming a note never rewrites the notes that mention it (D41).

import { createStore } from "solid-js/store";
import { ApiError, api } from "../api";

/** `undefined` while loading, `null` when the note does not exist or could not be read. */
const [titles, setTitles] = createStore<Record<string, string | null>>({});
const requested = new Set<string>();

/** The current title of note `id`, loading it on first use. Reactive. */
export function noteTitle(id: string): string | null | undefined {
  if (!requested.has(id)) {
    requested.add(id);
    void api.note(id).then(
      (n) => {
        const t = n.note.fields.title;
        setTitles(id, typeof t === "string" ? t : null);
      },
      (e) => {
        setTitles(id, null);
        if (!(e instanceof ApiError)) console.warn(`could not load the title of ${id}`, e);
      },
    );
  }
  return titles[id];
}

/** Record a title this client already knows, after a save or a search. */
export function knowTitle(id: string, title: string) {
  requested.add(id);
  setTitles(id, title);
}

/** Text for a mention: its label, else its target's title, else a placeholder. */
export function mentionText(id: string, label: string | null): string {
  if (label) return label;
  const t = noteTitle(id);
  if (t === undefined) return "…";
  return t ?? "Missing note";
}
