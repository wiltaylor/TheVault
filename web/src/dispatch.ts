// The threads that are about a note (TV-77, TV-78): notes sent to an agent with "Send to agent",
// and calls of an agent written in the note. `GET /api/dispatched?notes=<id>` gives each with
// where it stands, worked out by the server as it reads it: `active`, `needs_you` or `done`. A
// view watches a note while it shows it; the threads are read again on every `ai_turn` from the
// live stream, and every [`RUNNING_POLL_MS`] while one is active, since the apps may have no
// stream.

import { type Accessor, type Setter, createEffect, createRoot, createSignal, on, onCleanup } from "solid-js";
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

createRoot(() => {
  // A turn started or ended somewhere: the threads of the notes shown may have moved on.
  createEffect(
    on(
      turnsChanged,
      () => {
        for (const note of entries.keys()) void read(note);
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

/** Notes were sent: read their threads again, so the new thread shows at once. */
export function notesSent(notes: readonly string[]) {
  for (const note of notes) if (entries.has(note)) void read(note);
}

/** Test-only: forget every watched note. */
export function resetDispatched() {
  for (const e of entries.values()) clearTimeout(e.timer);
  entries.clear();
}
