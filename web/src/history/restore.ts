// Restoring a version from the History panel or a preview banner: after asking, the note's
// unsaved edits are saved, the version is written as a new one through the API, and the pane shows
// the live note with the restored content.

import { ApiError, api } from "../api";
import { when } from "../admin/format";
import { knowTitle } from "../editor/titles";
import { errorText } from "../pages/TypeEditor";
import { noteOf } from "../sidebars";
import { setPreview } from "./preview";

/** Restore version `v` of the note in `pane`. Resolves to an error to show, or null. */
export async function restoreVersion(pane: string, v: { noteId: string; version: string; at: number }): Promise<string | null> {
  if (!confirm(`Restore the version from ${when(v.at)}? It is saved as a new version; the current one stays in the history.`)) {
    return null;
  }
  const source = noteOf(pane);
  if (!source || source.noteId() !== v.noteId) return "The note is no longer open in this pane.";
  if (!(await source.flush())) return "Your latest edits could not be saved; resolve them before restoring.";
  try {
    const saved = await api.restoreVersion(v.noteId, v.version, source.version());
    const title = saved.note.fields.title;
    if (typeof title === "string") knowTitle(saved.id, title, saved.type);
    source.restored(saved);
    setPreview(pane, null);
    return null;
  } catch (e) {
    if (e instanceof ApiError && e.isConflict) return "The note changed since this list loaded. Try again.";
    return errorText(e);
  }
}
