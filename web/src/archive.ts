// The note the active pane shows, as far as archiving goes: its id, whether it is archived, and
// the page's own toggle, which saves pending edits first. The command palette offers "Archive
// this note" or "Unarchive this note" from it.

import { createRoot, createSignal } from "solid-js";
import { lit } from "./search/searchModel";

export interface ActiveNote {
  /** The page that set it, so only that page clears it. */
  owner: string;
  id: string;
  archived: boolean;
  /** Archive the note, or take it out of the archive, through the page that shows it. */
  toggle: () => Promise<void>;
}

const [active, setActive] = createRoot(() => createSignal<ActiveNote | null>(null));

/** Reactive: the note the active pane shows, or null when it shows none. */
export const activeNote = active;

/** The page showing `note` is in the active pane. */
export function setActiveNote(note: ActiveNote) {
  setActive(note);
}

/** The page `owner` left the active pane, or closed. */
export function clearActiveNote(owner: string) {
  if (active()?.owner === owner) setActive(null);
}

/**
 * The Archive page's query: archived notes of `type`, or of every type, newest change first and
 * newest note first within one second, narrowed by full-text `text`. As on the Search page, the
 * last word matches as a prefix until a space follows it.
 */
export function archiveQuery(text: string, type: string | null): string {
  const t = text.trim();
  const search = t ? ` search ${lit(/\s$/.test(text) ? `${t} ` : t)}` : "";
  return `from ${type || "*"} where archived${search} sort updated desc, id desc select updated`;
}
