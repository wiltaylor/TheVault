// The Dispatched page's model (TV-80, TV-77 D4): the sent and called threads in three sections,
// Active, Needs you and Done, and the dot a Done thread keeps until it is opened (TV-77 Q11).

import type { DispatchState, DispatchedThread } from "../api";
import { openedAt } from "../ai/opened";
import { hasNewReply, isViewing } from "../ai/running";

/** The sections of the page, in order. */
export const SECTIONS: { state: DispatchState; label: string; empty: string }[] = [
  { state: "active", label: "Active", empty: "No agent is working on sent notes now." },
  { state: "needs_you", label: "Needs you", empty: "Nothing waits for you." },
  { state: "done", label: "Done", empty: "Nothing is done yet." },
];

/** The threads of each state, in the order given (the server's: newest activity first). */
export function bySection(threads: DispatchedThread[]): Record<DispatchState, DispatchedThread[]> {
  const out: Record<DispatchState, DispatchedThread[]> = { active: [], needs_you: [], done: [] };
  for (const t of threads) (out[t.state] ?? out.done).push(t);
  return out;
}

/** When thread `t` was last active, in milliseconds, or 0 when unknown. */
export function activityMs(t: Pick<DispatchedThread, "last_activity">): number {
  const ms = t.last_activity ? Date.parse(t.last_activity) : Number.NaN;
  return Number.isNaN(ms) ? 0 : ms;
}

/**
 * Reactive: whether Done thread `t` has a dot. It has one when it is not open now and either it
 * answered while this tab ran and nobody had it open, or its last activity is later than when it
 * was last opened in this browser. `last_activity` is in whole seconds, so the opening is too.
 */
export function hasDot(t: Pick<DispatchedThread, "id" | "state" | "last_activity">): boolean {
  if (t.state !== "done" || isViewing(t.id)) return false;
  if (hasNewReply(t.id)) return true;
  return activityMs(t) > Math.floor(openedAt(t.id) / 1000) * 1000;
}

/** What a Needs you row says waits: the agent's question, or how many proposals. */
export function waitingText(t: Pick<DispatchedThread, "question" | "pending">): string {
  if (t.question) return t.question;
  if (t.pending > 0) return t.pending === 1 ? "1 proposal waits for you" : `${t.pending} proposals wait for you`;
  return "";
}

/** How the thread started, without presenting unattended triggers as sent notes. */
export function originText(t: Pick<DispatchedThread, "origin" | "notes">): string {
  if (t.origin === "mention") return "Called from a note";
  if (t.origin.startsWith("trigger:")) return `Trigger: ${t.origin.slice("trigger:".length)}`;
  const n = t.notes.length;
  return n === 1 ? "Sent 1 note" : `Sent ${n} notes`;
}

/** Where thread `id` of agent `agent` opens: the agent screen's chat, as `threadHref` in the panel. */
export function threadLink(agent: string, id: string): string {
  return `/agents/${encodeURIComponent(agent)}?thread=${encodeURIComponent(id)}`;
}
