// A count of the note writes this client has made, so live views can run their queries again
// after a save without waiting for their next poll.

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
