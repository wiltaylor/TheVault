// The side menu's Outbox badge: how many items wait for the user's review (outbox X7).

import { createSignal } from "solid-js";
import { api } from "../api";
import { onNoteChangedElsewhere, onNoteWritten } from "../changes";
import { REVIEW_QUERY } from "./model";

const [count, setCount] = createSignal<number | null>(null);

/** Reactive: how many Outbox items wait in Review, once counted. */
export const outboxCount = count;

let counting = 0;

/** Count the items in Review again. Only the latest count is kept when several overlap. */
export async function refreshOutboxCount(): Promise<void> {
  const mine = ++counting;
  try {
    const page = await api.query(REVIEW_QUERY, null, 1);
    if (mine === counting) setCount(page.total);
  } catch {
    // A vault without the type, or a failed request, shows no badge.
    if (mine === counting) setCount(null);
  }
}

onNoteWritten(() => void refreshOutboxCount());
// An agent's drafts, and approvals made in the apps, arrive from elsewhere.
onNoteChangedElsewhere(() => void refreshOutboxCount());
