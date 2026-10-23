// The writes the Outbox page and the note page make on Outbox items (outbox X7, X8).

import { type Json, type Note, type NoteDoc, api } from "../api";
import { knowTitle } from "../editor/titles";
import { applyEdits } from "../notes";
import { OUTBOX, type Status } from "./model";

/** Set item `id`'s status over its current version, with `extra` field changes. */
export async function setItemStatus(id: string, status: Status, extra: Record<string, Json | undefined> = {}): Promise<Note> {
  const n = await api.note(id);
  return api.updateNote(id, applyEdits(n.note, { status, ...extra }), n.version);
}

/** Create the Outbox item `doc` and return its note. */
export async function createDraft(doc: NoteDoc): Promise<Note> {
  const made = await api.createNote(OUTBOX, doc);
  const title = made.note.fields.title;
  if (typeof title === "string") knowTitle(made.id, title, made.type);
  return made;
}
