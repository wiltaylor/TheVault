// The side menu's Outbox badge: how many drafts an agent made, which wait on the user (outbox X7,
// X14).

import { createSignal } from "solid-js";
import { api } from "../api";
import { onNoteChangedElsewhere, onNoteWritten } from "../changes";
import { AGENT_DRAFTS_QUERY } from "./model";

const [count, setCount] = createSignal<number | null>(null);

/** Reactive: how many of an agent's drafts wait in the Outbox, once counted. */
export const outboxCount = count;

let counting = 0;

/** Count the agents' drafts again. Only the latest count is kept when several overlap. */
export async function refreshOutboxCount(): Promise<void> {
  const mine = ++counting;
  try {
    const page = await api.query(AGENT_DRAFTS_QUERY, null, 1);
    if (mine === counting) setCount(page.total);
  } catch {
    // A vault without the type, or a failed request, shows no badge.
    if (mine === counting) setCount(null);
  }
}

onNoteWritten(() => void refreshOutboxCount());
// An agent's drafts, and items marked ready in the apps, arrive from elsewhere.
onNoteChangedElsewhere(() => void refreshOutboxCount());
