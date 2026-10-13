// The notes pinned to the sidebar. Loaded from the server, and loaded again after this client
// pins, unpins, or writes a note, so titles and deletions show without a reload.

import { createResource, createSignal } from "solid-js";
import { type Pin, api } from "./api";
import { changes } from "./changes";
import { sessionState } from "./session";

const [pinChanges, setPinChanges] = createSignal(0);

const [pins, { mutate }] = createResource(
  // Nothing to load until signed in; a falsy source skips the fetch.
  () => sessionState() === "in" && ([changes(), pinChanges()] as const),
  // A failed load keeps the sidebar working with no pins rather than throwing on read.
  async () => api.pins().catch(() => [] as Pin[]),
  { initialValue: [] as Pin[] },
);

export { pins };

export function isPinned(id: string): boolean {
  return (pins() ?? []).some((p) => p.id === id);
}

/** Pin or unpin `id`, showing the change at once and then the server's list. */
export async function togglePin(id: string, title: string, type: string) {
  const pinned = isPinned(id);
  mutate((list) => (pinned ? list.filter((p) => p.id !== id) : [...list, { id, title, type }]));
  try {
    if (pinned) await api.unpin(id);
    else await api.pin(id);
  } finally {
    setPinChanges((n) => n + 1);
  }
}
