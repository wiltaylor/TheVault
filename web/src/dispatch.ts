// The threads that are about a note (TV-77, TV-78): notes sent to an agent with "Send to agent",
// and calls of an agent written in the note. `GET /api/dispatched?notes=<id>` gives each with
// where it stands, worked out by the server as it reads it: `active`, `needs_you` or `done`. A
// view watches a note while it shows it; the threads are read again on every `ai_turn` from the
// live stream, and every [`RUNNING_POLL_MS`] while one is active, since the apps may have no
// stream. A list (Inbox, Search) watches all of its rows at once with one read per page
// (TV-79), and groups the threads by note.

import { type Accessor, type Setter, createEffect, createMemo, createRoot, createSignal, on, onCleanup } from "solid-js";
import { RUNNING_POLL_MS, turnsChanged } from "./ai/running";
import { type DispatchedThread, api } from "./api";

interface Entry {
  threads: Accessor<DispatchedThread[]>;
  set: Setter<DispatchedThread[]>;
  /** How many views watch the note. */
  users: number;
  timer?: ReturnType<typeof setTimeout>;
  busy: boolean;
  /** Something changed while a read was on its way: read once more after it. */
  again: boolean;
}

const entries = new Map<string, Entry>();

/** The most note ids `GET /api/dispatched` filters by in one call (`MAX_FILTER_NOTES`). */
export const FILTER_MAX = 500;

interface ListEntry {
  /** The notes the list shows, in its order. */
  ids: readonly string[];
  set: Setter<ReadonlyMap<string, DispatchedThread[]>>;
  timer?: ReturnType<typeof setTimeout>;
  busy: boolean;
  again: boolean;
}

const lists = new Set<ListEntry>();

async function read(note: string) {
  const e = entries.get(note);
  if (!e) return;
  if (e.busy) {
    e.again = true;
    return;
  }
  e.busy = true;
  e.again = false;
  clearTimeout(e.timer);
  e.timer = undefined;
  try {
    const got = await api.dispatched({ notes: [note] });
    e.set(got.threads);
    if (entries.get(note) === e && got.threads.some((t) => t.state === "active")) {
      e.timer = setTimeout(() => void read(note), RUNNING_POLL_MS);
    }
  } catch (err) {
    console.warn(`threads of ${note}:`, err);
  } finally {
    e.busy = false;
    if (e.again && entries.get(note) === e) void read(note);
  }
}

/**
 * The threads of `threads` about each of `ids`, newest activity first as given. A thread about
 * several of the notes is under each of them, once.
 */
export function threadsByNote(ids: readonly string[], threads: readonly DispatchedThread[]): Map<string, DispatchedThread[]> {
  const want = new Set(ids);
  const seen = new Set<string>();
  const out = new Map<string, DispatchedThread[]>();
  for (const t of threads) {
    if (seen.has(t.id)) continue;
    seen.add(t.id);
    for (const note of new Set(t.notes)) {
      if (!want.has(note)) continue;
      const list = out.get(note);
      if (list) list.push(t);
      else out.set(note, [t]);
    }
  }
  return out;
}

async function readList(e: ListEntry) {
  if (e.busy) {
    e.again = true;
    return;
  }
  e.busy = true;
  e.again = false;
  clearTimeout(e.timer);
  e.timer = undefined;
  const ids = e.ids;
  try {
    if (ids.length === 0) {
      e.set(new Map());
      return;
    }
    const parts: string[][] = [];
    for (let i = 0; i < ids.length; i += FILTER_MAX) parts.push(ids.slice(i, i + FILTER_MAX));
    const got = await Promise.all(parts.map((notes) => api.dispatched({ notes })));
    const threads = got.flatMap((g) => g.threads);
    if (!lists.has(e) || e.ids !== ids) return;
    e.set(threadsByNote(ids, threads));
    if (threads.some((t) => t.state === "active")) e.timer = setTimeout(() => void readList(e), RUNNING_POLL_MS);
  } catch (err) {
    console.warn("threads of the listed notes:", err);
  } finally {
    e.busy = false;
    if (lists.has(e) && (e.again || e.ids !== ids)) void readList(e);
  }
}

createRoot(() => {
  // A turn started or ended somewhere: the threads of the notes shown may have moved on.
  createEffect(
    on(
      turnsChanged,
      () => {
        for (const note of entries.keys()) void read(note);
        for (const list of lists) void readList(list);
      },
      { defer: true },
    ),
  );
});

/**
 * Reactive: the threads about note `note`, newest activity first. Call it in a component: the
 * note is watched until the component is cleaned up.
 */
export function watchNoteThreads(note: Accessor<string>): Accessor<DispatchedThread[]> {
  const [current, setCurrent] = createSignal<Entry | null>(null);
  createEffect(
    on(note, (id) => {
      let e = entries.get(id);
      if (!e) {
        const [threads, set] = createRoot(() => createSignal<DispatchedThread[]>([]));
        e = { threads, set, users: 0, busy: false, again: false };
        entries.set(id, e);
        void read(id);
      }
      e.users++;
      setCurrent(e);
      onCleanup(() => {
        e.users--;
        if (e.users <= 0) {
          clearTimeout(e.timer);
          entries.delete(id);
        }
      });
    }),
  );
  return () => current()?.threads() ?? [];
}

/**
 * Reactive: the threads about each note of a list, by note id, read for the whole list in one
 * call (TV-79) whenever the notes shown change. Call it in a component: the list is watched
 * until the component is cleaned up.
 */
export function watchListThreads(ids: Accessor<readonly string[]>): Accessor<ReadonlyMap<string, DispatchedThread[]>> {
  const [threads, set] = createSignal<ReadonlyMap<string, DispatchedThread[]>>(new Map());
  const e: ListEntry = { ids: [], set, busy: false, again: false };
  lists.add(e);
  onCleanup(() => {
    clearTimeout(e.timer);
    lists.delete(e);
  });
  // Read only when the notes shown change, not each time the list is computed again.
  const key = createMemo(() => ids().join(","));
  createEffect(
    on(
      key,
      (key) => {
        e.ids = key ? key.split(",") : [];
        void readList(e);
      },
    ),
  );
  return threads;
}

/** Notes were sent: read their threads again, so the new thread shows at once. */
export function notesSent(notes: readonly string[]) {
  for (const note of notes) if (entries.has(note)) void read(note);
  for (const list of lists) if (notes.some((n) => list.ids.includes(n))) void readList(list);
}

/** Test-only: forget every watched note and list. */
export function resetDispatched() {
  for (const e of entries.values()) clearTimeout(e.timer);
  entries.clear();
  for (const l of lists) clearTimeout(l.timer);
  lists.clear();
}
