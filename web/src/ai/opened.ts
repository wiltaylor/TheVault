// When each agent thread was last opened in this browser (TV-80, TV-77 Q11), so a sent or called
// thread that ended while nobody looked keeps a dot until it is opened, also when it ended while
// the browser was closed. The map is kept per browser: the phone and the web app each have their
// own. A thread never opened here counts as opened when the map was started, so threads that were
// done before this browser first kept the map get no dot.

import { createRoot, createSignal } from "solid-js";

export const OPENED_KEY = "thevault.ai.opened";

interface Opened {
  /** When this browser started the map, in milliseconds since the epoch. */
  since: number;
  /** When each thread was last opened or closed, by id. */
  at: Record<string, number>;
}

/** The most threads the map keeps; the ones opened longest ago go first. */
const MAX_KEPT = 500;

function load(): Opened {
  try {
    const raw = JSON.parse(localStorage.getItem(OPENED_KEY) ?? "null") as Partial<Opened> | null;
    if (raw && typeof raw.since === "number" && raw.at && typeof raw.at === "object") {
      const at: Record<string, number> = {};
      for (const [k, v] of Object.entries(raw.at)) if (typeof v === "number") at[k] = v;
      return { since: raw.since, at };
    }
  } catch {
    // A broken value starts again.
  }
  const fresh = { since: Date.now(), at: {} };
  save(fresh);
  return fresh;
}

function save(o: Opened) {
  try {
    localStorage.setItem(OPENED_KEY, JSON.stringify(o));
  } catch {
    // Private browsing: the map lasts until the page closes.
  }
}

const store = createRoot(() => {
  const [opened, setOpened] = createSignal<Opened>(load());
  return { opened, setOpened };
});

/** Thread `id` is opened or closed now. */
export function markOpened(id: string, now = Date.now()) {
  const cur = store.opened();
  let at = { ...cur.at, [id]: now };
  const ids = Object.keys(at);
  if (ids.length > MAX_KEPT) {
    ids.sort((a, b) => at[a]! - at[b]!);
    at = Object.fromEntries(ids.slice(ids.length - MAX_KEPT).map((k) => [k, at[k]!]));
  }
  const next = { since: cur.since, at };
  store.setOpened(next);
  save(next);
}

/** Reactive: when thread `id` was last opened here, or when the map started for one never opened. */
export function openedAt(id: string): number {
  const o = store.opened();
  return o.at[id] ?? o.since;
}

/** Test-only: start the map again at `since`. */
export function resetOpened(since = Date.now()) {
  const fresh = { since, at: {} };
  store.setOpened(fresh);
  save(fresh);
}
