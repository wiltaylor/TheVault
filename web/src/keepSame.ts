// Lists read again from the server keep the objects that did not change, so a read caused by a
// change elsewhere leaves open views of everything else as they are: a memo, `<For>` or `<Show>`
// over an unchanged type or agent sees the same object and redraws nothing.

/**
 * `next`, with each item that equals the item of the same key in `prev` replaced by that item,
 * and `prev` itself when nothing changed at all.
 */
export function keepSame<T>(prev: readonly T[], next: readonly T[], key: (item: T) => string): T[] {
  const old = new Map(prev.map((item) => [key(item), item]));
  let changed = prev.length !== next.length;
  const out = next.map((item, i) => {
    const before = old.get(key(item));
    if (before !== undefined && JSON.stringify(before) === JSON.stringify(item)) {
      if (prev[i] !== before) changed = true;
      return before;
    }
    changed = true;
    return item;
  });
  return changed ? out : (prev as T[]);
}
