// A count of the note writes this client has made, so live views can run their queries again
// after a save without waiting for their next poll.

import { createSignal } from "solid-js";

const [changes, setChanges] = createSignal(0);

/** Reactive: bumped after every note write this client makes. */
export { changes };

export function noteWritten() {
  setChanges((n) => n + 1);
}
