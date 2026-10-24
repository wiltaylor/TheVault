// The writes the Outbox page and the note page make on Outbox items (outbox X7, X8).

import { type Json, type Note, type NoteDoc, api } from "../api";
import { knowTitle } from "../editor/titles";
import { applyEdits } from "../notes";
import { OUTBOX, type Status, messageParties, smsReplyDraft, smsTargets, str, strs } from "./model";

/** Set item `id`'s status over its current version, with `extra` field changes. */
export async function setItemStatus(id: string, status: Status, extra: Record<string, Json | undefined> = {}): Promise<Note> {
  const n = await api.note(id);
  return api.updateNote(id, applyEdits(n.note, { status, ...extra }), n.version);
}

/** Note `id`'s content, or null when it cannot be read. */
async function readDoc(id: string): Promise<NoteDoc | null> {
  try {
    return (await api.note(id)).note;
  } catch {
    return null;
  }
}

/** The connected Google account's address, or null when none is connected or it cannot be read. */
export async function googleAccount(): Promise<string | null> {
  try {
    return (await api.googleStatus()).account ?? null;
  } catch {
    return null;
  }
}

/**
 * The people of message note `message`'s conversation (outbox X13), its `participants`, or none
 * when it has no conversation.
 */
export async function conversationPeople(message: NoteDoc): Promise<string[]> {
  const conversation = str(message.fields, "conversation");
  if (!conversation) return [];
  const doc = await readDoc(conversation);
  return doc ? strs(doc.fields, "participants") : [];
}

/**
 * The SMS drafts answering message note `message` (outbox X13): one to the other party, or with
 * `all` one per other number of the message and of its conversation's participants.
 */
export async function smsReplyDrafts(message: NoteDoc, all: boolean): Promise<NoteDoc[]> {
  const { raws, people } = messageParties(message, all);
  const ids = [...new Set(all ? people.concat(await conversationPeople(message)) : people)];
  const docs = await Promise.all(ids.map(async (id) => ({ doc: await readDoc(id) })));
  const targets = smsTargets(raws, docs);
  if (!targets.length) {
    const name = raws.find(Boolean) ?? str(docs[0]?.doc?.fields ?? {}, "title");
    return [smsReplyDraft({ number: "", name })];
  }
  return (all ? targets : targets.slice(0, 1)).map(smsReplyDraft);
}

/** Create the Outbox item `doc` and return its note. */
export async function createDraft(doc: NoteDoc): Promise<Note> {
  const made = await api.createNote(OUTBOX, doc);
  const title = made.note.fields.title;
  if (typeof title === "string") knowTitle(made.id, title, made.type);
  return made;
}
