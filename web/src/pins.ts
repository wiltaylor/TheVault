// The sidebar's pinned tree. Loaded from the server, and loaded again after this client changes
// it or writes a note, so titles and deletions show without a reload. Every change shows at once
// and is then replaced by the server's tree.

import { createResource, createSignal } from "solid-js";
import { type PinItem, api } from "./api";
import { changes } from "./changes";
import { appendEntry, findItem, moveEntry, removeLifting } from "./pinTree";
import { sessionState } from "./session";

const [pinChanges, setPinChanges] = createSignal(0);

const [pins, { mutate }] = createResource(
  // Nothing to load until signed in; a falsy source skips the fetch.
  () => sessionState() === "in" && ([changes(), pinChanges()] as const),
  // A failed load keeps the sidebar working with no pins rather than throwing on read.
  async () => api.pins().catch(() => [] as PinItem[]),
  { initialValue: [] as PinItem[] },
);

export { pins };

export function isPinned(id: string): boolean {
  const item = findItem(pins() ?? [], id);
  return item?.kind === "note";
}

/** Show `optimistic` at once, run `write`, then load the server's tree whatever happened. */
async function change<T>(optimistic: ((tree: PinItem[]) => PinItem[]) | null, write: () => Promise<T>): Promise<T> {
  if (optimistic) mutate((tree) => optimistic(tree ?? []));
  try {
    return await write();
  } finally {
    setPinChanges((n) => n + 1);
  }
}

/** Pin or unpin `id`, showing the change at once and then the server's tree. */
export async function togglePin(id: string, title: string, type: string) {
  if (isPinned(id)) await unpin(id);
  else
    await change(
      (tree) => appendEntry(tree, null, { kind: "note", id, title, type, children: [] }),
      () => api.pin(id),
    );
}

/** Unpin a note; the entries under it move up into its place. */
export function unpin(id: string) {
  return change((tree) => removeLifting(tree, id), () => api.unpin(id));
}

export function createFolder(name: string, parent: string | null = null) {
  return change(null, () => api.createPinFolder(name, parent));
}

export function renameFolder(id: string, name: string) {
  const rename = (tree: PinItem[]): PinItem[] =>
    tree.map((i) => ({ ...(i.id === id && i.kind === "folder" ? { ...i, name } : i), children: rename(i.children) }));
  return change(rename, () => api.renamePinFolder(id, name));
}

/** Delete a folder; the entries in it move up into its place. */
export function deleteFolder(id: string) {
  return change((tree) => removeLifting(tree, id), () => api.deletePinFolder(id));
}

/** Move entry `id` under `parent` (null for the top level) to `index` among its children. */
export function movePin(id: string, parent: string | null, index: number) {
  return change((tree) => moveEntry(tree, id, parent, index) ?? tree, () => api.movePin(id, parent, index));
}

const EXPANDED_KEY = "thevault.pins.expanded";

function loadExpanded(): Set<string> {
  try {
    const raw = JSON.parse(localStorage.getItem(EXPANDED_KEY) ?? "[]");
    return new Set(Array.isArray(raw) ? raw.filter((x) => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

const [expanded, setExpanded] = createSignal(loadExpanded());

/** Whether a folder or a note with children is open. Remembered per browser. */
export function isExpanded(id: string): boolean {
  return expanded().has(id);
}

export function setExpandedFor(id: string, open: boolean) {
  const next = new Set(expanded());
  if (open) next.add(id);
  else next.delete(id);
  setExpanded(next);
  try {
    localStorage.setItem(EXPANDED_KEY, JSON.stringify([...next]));
  } catch {
    // Storage may be full or blocked; the state still holds for this page.
  }
}
