// A count of the note writes this client has made, so live views can run their queries again
// after a save without waiting for their next poll; and the notes the server's live stream says
// changed elsewhere, so an open note can take the change.

import { createSignal } from "solid-js";

const [changes, setChanges] = createSignal(0);

/** Reactive: bumped after every note write this client makes. */
export { changes };

const listeners = new Set<() => void>();

/** Run `fn` after every note write this client makes. Returns a function that stops it. */
export function onNoteWritten(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function noteWritten() {
  setChanges((n) => n + 1);
  for (const fn of listeners) fn();
}

/** A note that changed elsewhere and its new version, or `null` when any note may have. */
export type ElsewhereChange = { id: string; version: string } | null;

const elsewhere = new Set<(change: ElsewhereChange) => void>();

/**
 * Run `fn` when the server's live stream reports a note changed somewhere else (another device, a
 * script, the CLI), with `null` after the stream was down and any note may have. Returns a
 * function that stops it.
 */
export function onNoteChangedElsewhere(fn: (change: ElsewhereChange) => void): () => void {
  elsewhere.add(fn);
  return () => elsewhere.delete(fn);
}

export function noteChangedElsewhere(change: ElsewhereChange) {
  for (const fn of elsewhere) fn(change);
}
