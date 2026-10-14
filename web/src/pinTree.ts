// Pure operations on the sidebar's pinned tree, mirroring what the server does, so the sidebar
// can show a change at once and then load the server's tree.

import type { PinItem } from "./api";

/** Where an entry sits: its parent's id (null for the top level) and its index there. */
export interface Place {
  parent: string | null;
  index: number;
}

/** Where a dragged entry would land: before or after `target` as a sibling, or inside it. */
export type DropZone = "before" | "after" | "inside";

export function findItem(tree: PinItem[], id: string): PinItem | undefined {
  for (const item of tree) {
    if (item.id === id) return item;
    const found = findItem(item.children, id);
    if (found) return found;
  }
  return undefined;
}

export function placeOf(tree: PinItem[], id: string, parent: string | null = null): Place | undefined {
  for (const [index, item] of tree.entries()) {
    if (item.id === id) return { parent, index };
    const found = placeOf(item.children, id, item.id);
    if (found) return found;
  }
  return undefined;
}

/** Whether `id` is `ancestor` itself or nested anywhere under it. */
export function isWithin(tree: PinItem[], ancestor: string, id: string): boolean {
  if (ancestor === id) return true;
  const a = findItem(tree, ancestor);
  return !!a && findItem(a.children, id) !== undefined;
}

/** `tree` without entry `id`, its children moved up into its place. */
export function removeLifting(tree: PinItem[], id: string): PinItem[] {
  return tree.flatMap((item) =>
    item.id === id ? removeLifting(item.children, id) : [{ ...item, children: removeLifting(item.children, id) }],
  );
}

function withoutSubtree(tree: PinItem[], id: string): PinItem[] {
  return tree.filter((i) => i.id !== id).map((i) => ({ ...i, children: withoutSubtree(i.children, id) }));
}

function insertAt(tree: PinItem[], parent: string | null, index: number, entry: PinItem): PinItem[] {
  if (parent === null) {
    const out = [...tree];
    out.splice(Math.min(Math.max(index, 0), out.length), 0, entry);
    return out;
  }
  return tree.map((i) =>
    i.id === parent
      ? { ...i, children: insertAt(i.children, null, index, entry) }
      : { ...i, children: insertAt(i.children, parent, index, entry) },
  );
}

/** Append `entry` under `parent`, or at the end of the top level. */
export function appendEntry(tree: PinItem[], parent: string | null, entry: PinItem): PinItem[] {
  return insertAt(tree, parent, Number.MAX_SAFE_INTEGER, entry);
}

/**
 * `tree` with `id` moved under `parent` at `index`, counted without the moved entry and clamped,
 * as `POST /api/pins/move` does. A move into the entry itself or its own descendant gives
 * `undefined`.
 */
export function moveEntry(tree: PinItem[], id: string, parent: string | null, index: number): PinItem[] | undefined {
  const entry = findItem(tree, id);
  if (!entry) return undefined;
  if (parent !== null && (isWithin(tree, id, parent) || !findItem(tree, parent))) return undefined;
  return insertAt(withoutSubtree(tree, id), parent, index, entry);
}

/**
 * The move a drop of `dragged` on `target` in `zone` asks for, or `undefined` when the drop is
 * refused (into the entry itself or its own descendant) or would change nothing.
 */
export function dropMove(tree: PinItem[], dragged: string, target: string | null, zone: DropZone): Place | undefined {
  const from = placeOf(tree, dragged);
  if (!from) return undefined;
  let to: Place;
  if (target === null) {
    to = { parent: null, index: tree.filter((i) => i.id !== dragged).length };
  } else {
    if (target === dragged) return undefined;
    const t = placeOf(tree, target);
    const targetItem = findItem(tree, target);
    if (!t || !targetItem) return undefined;
    if (zone === "inside") {
      to = { parent: target, index: targetItem.children.filter((i) => i.id !== dragged).length };
    } else {
      // Count the target's index without the dragged entry, as the server does.
      const sameParent = from.parent === t.parent && from.index < t.index;
      const base = sameParent ? t.index - 1 : t.index;
      to = { parent: t.parent, index: zone === "before" ? base : base + 1 };
    }
  }
  if (to.parent !== null && isWithin(tree, dragged, to.parent)) return undefined;
  if (to.parent === from.parent && to.index === from.index) return undefined;
  return to;
}

/** Every folder, depth first, with its depth, for the "Move to…" menu. */
export function folders(tree: PinItem[], depth = 0): { id: string; name: string; depth: number }[] {
  return tree.flatMap((i) => [
    ...(i.kind === "folder" ? [{ id: i.id, name: i.name, depth }] : []),
    ...folders(i.children, i.kind === "folder" ? depth + 1 : depth),
  ]);
}
