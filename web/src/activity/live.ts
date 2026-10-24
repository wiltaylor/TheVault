// The activity log's live entries (activity Y3): the server publishes each new entry on the live
// event stream as `activity`, and the Log page listens here. When the stream opens again after a
// drop, entries may have been missed, so listeners are told to read the newest page again.

import type { ActivityItem } from "../api";

type Listener = (item: ActivityItem) => void;

const listeners = new Set<Listener>();
const resyncs = new Set<() => void>();

/** Call `fn` with every new entry until the returned function is called. */
export function onActivity(fn: Listener): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Call `fn` when the stream opens again after a drop, until the returned function is called. */
export function onActivityResync(fn: () => void): () => void {
  resyncs.add(fn);
  return () => resyncs.delete(fn);
}

/** An `activity` event's data: the entry, or `{item}` around it. */
export function activityOf(data: unknown): ActivityItem | null {
  const d = data && typeof data === "object" && "item" in data ? (data as { item: unknown }).item : data;
  if (!d || typeof d !== "object") return null;
  const item = d as Partial<ActivityItem>;
  if (typeof item.id !== "string" || typeof item.kind !== "string") return null;
  return d as ActivityItem;
}

/** The stream sent an `activity` event. */
export function liveActivity(data: unknown) {
  const item = activityOf(data);
  if (!item) return;
  for (const fn of [...listeners]) fn(item);
}

/** The stream opened again after a drop. */
export function activityResync() {
  for (const fn of [...resyncs]) fn();
}
