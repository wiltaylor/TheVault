// Which agent threads have a turn running, and which answered while nobody had them open
// (agents G17). The server announces each turn on a thread starting and ending on the live
// stream as `ai_turn`; the agent screen's session picker and the sidebar's agents read again when it
// does, and the apps, which may not have the stream, poll while something runs. A thread whose
// turn ended while no conversation showed it gets a "New reply" dot until it is opened, kept per
// browser.

import { createRoot, createSignal } from "solid-js";

export const NEW_REPLIES_KEY = "thevault.ai.new_replies";

/**
 * When this browser last opened each thread, in milliseconds since the epoch (TV-77 Q11): a
 * thread sent notes or called from a note that is done keeps its dot until it is opened after its
 * last activity. Kept per browser, so the phone and the web are separate.
 */
export const OPENED_KEY = "thevault.ai.opened";

/** The most threads the opened map keeps; the oldest openings go first. */
export const OPENED_MAX = 500;

/** How often a thread list reads again while a thread of it runs (agents G17). */
export const RUNNING_POLL_MS = 4_000;

/** What `ai_turn` says. */
export interface AiTurnEvent {
  agent: string;
  thread: string;
  turn: string | null;
  running: boolean;
}

function loadNew(): ReadonlySet<string> {
  try {
    const raw = JSON.parse(localStorage.getItem(NEW_REPLIES_KEY) ?? "[]") as unknown;
    return new Set(Array.isArray(raw) ? raw.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function loadOpened(): Readonly<Record<string, number>> {
  try {
    const raw = JSON.parse(localStorage.getItem(OPENED_KEY) ?? "{}") as unknown;
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
    const out: Record<string, number> = {};
    for (const [id, at] of Object.entries(raw)) if (typeof at === "number" && Number.isFinite(at)) out[id] = at;
    return out;
  } catch {
    return {};
  }
}

const store = createRoot(() => {
  const [changes, setChanges] = createSignal(0);
  const [last, setLast] = createSignal<AiTurnEvent | null>(null);
  const [fresh, setFresh] = createSignal<ReadonlySet<string>>(loadNew());
  const [opened, setOpened] = createSignal<Readonly<Record<string, number>>>(loadOpened());
  return { changes, setChanges, last, setLast, fresh, setFresh, opened, setOpened };
});

/** Reactive: bumped by every turn starting or ending, as the live stream or this tab saw it. */
export const turnsChanged = store.changes;

/** Reactive: the last `ai_turn` seen, or null. */
export const lastTurnEvent = store.last;

/** Something may have started or ended: lists showing running state read again. */
export function bumpTurns(event: AiTurnEvent | null = null) {
  if (event) store.setLast(event);
  store.setChanges(store.changes() + 1);
}

/** The threads conversations show now, with how many show each. */
const viewing = new Map<string, number>();

/** A conversation shows thread `id` until the returned function is called; it has no new reply. */
export function viewThread(id: string): () => void {
  viewing.set(id, (viewing.get(id) ?? 0) + 1);
  seen(id);
  markOpened(id);
  let done = false;
  return () => {
    if (done) return;
    done = true;
    const n = (viewing.get(id) ?? 1) - 1;
    if (n > 0) viewing.set(id, n);
    else viewing.delete(id);
  };
}

/** Whether a conversation shows thread `id` now. */
export function isViewing(id: string): boolean {
  return (viewing.get(id) ?? 0) > 0;
}

function saveNew(next: ReadonlySet<string>) {
  store.setFresh(next);
  try {
    localStorage.setItem(NEW_REPLIES_KEY, JSON.stringify([...next]));
  } catch {
    // Private browsing: the dots last until the page closes.
  }
}

/** Reactive: whether thread `id` answered while nobody had it open. */
export function hasNewReply(id: string): boolean {
  return store.fresh().has(id);
}

/** Thread `id` was opened: its new reply is seen. */
export function seen(id: string) {
  if (!store.fresh().has(id)) return;
  const next = new Set(store.fresh());
  next.delete(id);
  saveNew(next);
}

/** The turn on thread `id` ended: unless a conversation shows it, it has a new reply. */
export function turnEnded(id: string) {
  if (isViewing(id)) {
    // Watched as it answered: opened after its last activity.
    markOpened(id);
    return;
  }
  if (store.fresh().has(id)) return;
  saveNew(new Set([...store.fresh(), id]));
}

/** An `ai_turn` from the live stream. */
export function liveTurn(data: unknown) {
  const d = (data ?? {}) as Partial<AiTurnEvent>;
  if (typeof d.thread !== "string" || typeof d.agent !== "string") return;
  const event: AiTurnEvent = { agent: d.agent, thread: d.thread, turn: typeof d.turn === "string" ? d.turn : null, running: d.running === true };
  if (!event.running) turnEnded(event.thread);
  bumpTurns(event);
}

/**
 * Thread `id` was opened now. `lastActivity`, the thread's last activity as the server gave it,
 * counts as the time when it is later than this browser's clock, so a clock behind the server's
 * does not bring the dot back.
 */
export function markOpened(id: string, lastActivity?: string | null) {
  const known = lastActivity ? Date.parse(lastActivity) : NaN;
  const at = Math.max(Date.now(), Number.isFinite(known) ? known : 0);
  const entries = Object.entries(store.opened()).filter(([k]) => k !== id);
  entries.push([id, at]);
  entries.sort((a, b) => a[1] - b[1]);
  const next = Object.fromEntries(entries.slice(-OPENED_MAX));
  store.setOpened(next);
  try {
    localStorage.setItem(OPENED_KEY, JSON.stringify(next));
  } catch {
    // Private browsing: the openings last until the page closes.
  }
}

/** Reactive: when this browser last opened thread `id`, or null when it never did. */
export function openedAt(id: string): number | null {
  return store.opened()[id] ?? null;
}

/** Reactive: whether thread `id`, last active at `lastActivity`, has activity this browser has not opened. */
export function unopened(id: string, lastActivity: string | null | undefined): boolean {
  const at = openedAt(id);
  if (at === null) return true;
  const last = lastActivity ? Date.parse(lastActivity) : NaN;
  return Number.isFinite(last) && last > at;
}

/** Test-only: forget every view and new reply. */
export function resetRunning() {
  viewing.clear();
  saveNew(new Set());
  store.setOpened({});
  try {
    localStorage.removeItem(OPENED_KEY);
  } catch {
    // Nothing kept.
  }
  store.setLast(null);
}
