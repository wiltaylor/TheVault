// Calls of agents committed in notes (agents G19). The editor's `@` picker commits a call as one
// link, `[@<Label> <request>](agent:<name>?call=<ULID>)`, with a call id made here, and the
// note's save carries it whole. The server answers each call id once and never rewrites the
// note; `GET /api/agents/calls/{call}` says what became of a call: 404 while the server has not
// seen it yet, then its thread (null for a call it did not run) and whether the thread runs.

import { type Accessor, type Setter, createEffect, createRoot, createSignal, on } from "solid-js";
import { RUNNING_POLL_MS, turnsChanged } from "./ai/running";
import { ApiError, api } from "./api";

/** The URL of a call: the agent's name and the call's ULID. */
const CALL_HREF = /^agent:([a-z][a-z0-9_]{0,63})\?call=([0-9A-HJKMNP-TV-Z]{26})$/;

/** The agent and call id an `agent:<name>?call=<ULID>` URL names, or null for any other URL. */
export function parseAgentCall(href: string): { agent: string; call: string } | null {
  const m = CALL_HREF.exec(href);
  return m ? { agent: m[1]!, call: m[2]! } : null;
}

/** The URL of call `call` of agent `agent`. */
export function agentCallHref(agent: string, call: string): string {
  return `agent:${agent}?call=${call}`;
}

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/** A new ULID: 48 bits of the time in milliseconds, then 80 random bits, in Crockford base 32. */
export function newUlid(now: number = Date.now()): string {
  let time = "";
  let t = Math.max(0, Math.floor(now));
  for (let i = 0; i < 10; i++) {
    time = CROCKFORD.charAt(t % 32) + time;
    t = Math.floor(t / 32);
  }
  const bytes = new Uint8Array(10);
  crypto.getRandomValues(bytes);
  // 80 bits as 16 characters of 5 bits each.
  let bits = 0;
  let value = 0;
  let random = "";
  for (const b of bytes) {
    value = (value << 8) | b;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      random += CROCKFORD.charAt((value >> bits) & 31);
    }
    value &= (1 << bits) - 1;
  }
  return time + random;
}

/** What became of a call, as far as this tab knows. */
export interface CallStatus {
  /** `sending` until the server has seen the call; `answered` with a thread; `unanswered` for one it did not run. */
  state: "sending" | "answered" | "unanswered";
  thread: string | null;
  running: boolean;
}

const SENDING: CallStatus = { state: "sending", thread: null, running: false };

/** How often a call the server has not seen yet is asked about, and for how long. */
export const CALL_POLL_MS = 3_000;
export const CALL_POLL_FOR_MS = 60_000;

interface Entry {
  status: Accessor<CallStatus>;
  set: Setter<CallStatus>;
  since: number;
  timer?: ReturnType<typeof setTimeout>;
  busy: boolean;
}

const entries = new Map<string, Entry>();

async function ask(call: string) {
  const e = entries.get(call);
  if (!e || e.busy) return;
  e.busy = true;
  clearTimeout(e.timer);
  e.timer = undefined;
  try {
    const got = await api.agentCall(call);
    e.set({ state: got.thread ? "answered" : "unanswered", thread: got.thread, running: got.running });
    // The apps may not have the live stream: while the thread runs, ask again now and then.
    if (got.running) e.timer = setTimeout(() => void ask(call), RUNNING_POLL_MS);
  } catch (err) {
    if (!(err instanceof ApiError && err.status === 404)) console.warn(`call ${call}:`, err);
    // Not seen yet: the note's save may still be on its way.
    if (Date.now() - e.since < CALL_POLL_FOR_MS) e.timer = setTimeout(() => void ask(call), CALL_POLL_MS);
  } finally {
    e.busy = false;
  }
}

createRoot(() => {
  // A turn started or ended somewhere: the calls answered may be running or done.
  createEffect(
    on(
      turnsChanged,
      () => {
        for (const [call, e] of entries) if (e.status().state !== "unanswered") void ask(call);
      },
      { defer: true },
    ),
  );
});

/** Reactive: what became of call `call`, asked of the server the first time a view wants it. */
export function callStatus(call: string): CallStatus {
  let e = entries.get(call);
  if (!e) {
    const [status, set] = createRoot(() => createSignal<CallStatus>(SENDING));
    e = { status, set, since: Date.now(), busy: false };
    entries.set(call, e);
    void ask(call);
  }
  return e.status();
}

/** A note holding calls was saved: ask again about those the server had not seen. */
export function callsSaved() {
  for (const [call, e] of entries) {
    if (e.status().state === "sending") {
      e.since = Date.now();
      void ask(call);
    }
  }
}

/** Where clicking call `call` of `agent` goes: its thread, or the agent's chat until it has one. */
export function callTarget(agent: string, call: string): string {
  const thread = entries.get(call)?.status().thread;
  return thread ? `/agents/${agent}?thread=${thread}` : `/agents/${agent}`;
}

/** Test-only: forget every call. */
export function resetCalls() {
  for (const e of entries.values()) clearTimeout(e.timer);
  entries.clear();
}
