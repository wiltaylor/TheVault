// The version of a note a pane is previewing, chosen in the History panel. While a pane has one,
// its note page shows that version read-only, with a banner, and does not save.

import { createRoot, createSignal } from "solid-js";
import type { Author, Note } from "../api";

export interface Preview {
  noteId: string;
  version: string;
  note: Note;
  author: Author;
  at: number;
}

const [previews, setPreviews] = createRoot(() => createSignal<Record<string, Preview>>({}));

/** Reactive: the version `pane` previews, if any. */
export function previewOf(pane: string): Preview | undefined {
  return previews()[pane];
}

/** Show `preview` in `pane`, or with null, the live note again. */
export function setPreview(pane: string, preview: Preview | null) {
  setPreviews((all) => {
    if (!preview && !(pane in all)) return all;
    const next = { ...all };
    if (preview) next[pane] = preview;
    else delete next[pane];
    return next;
  });
}

/** End every preview except the one in `keep`, if given. */
export function endPreviews(keep?: string) {
  setPreviews((all) => {
    const panes = Object.keys(all);
    if (panes.every((p) => p === keep)) return all;
    return keep && all[keep] ? { [keep]: all[keep]! } : {};
  });
}
