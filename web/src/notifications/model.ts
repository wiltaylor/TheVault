// What the notifications panel shows, worked out from the API's answers: the badge, the time and
// source of each item, where it goes, and the list after each change.

import type { AppNotification } from "../api";

/** The unread badge's text: nothing at 0, then the count up to 99, then "99+". */
export function badgeText(unread: number): string {
  if (!Number.isFinite(unread) || unread <= 0) return "";
  return unread > 99 ? "99+" : String(Math.floor(unread));
}

/** The bell button's accessible name, with the unread count. */
export function bellLabel(unread: number): string {
  if (unread <= 0) return "Notifications";
  return `Notifications, ${unread} unread`;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** How long ago `ms` was, briefly: "just now", "5 min ago", "3 h ago", "yesterday", "4 d ago", then the date. */
export function relativeTime(ms: number, now: number): string {
  const ago = now - ms;
  if (ago < MINUTE) return "just now";
  if (ago < HOUR) return `${Math.floor(ago / MINUTE)} min ago`;
  if (ago < DAY) return `${Math.floor(ago / HOUR)} h ago`;
  if (ago < 2 * DAY) return "yesterday";
  if (ago < 7 * DAY) return `${Math.floor(ago / DAY)} d ago`;
  const d = new Date(ms);
  const day = `${d.getDate()} ${MONTHS[d.getMonth()]}`;
  return d.getFullYear() === new Date(now).getFullYear() ? day : `${day} ${d.getFullYear()}`;
}

/** Who a notification is from, for its label: a task's or token's name, a handler's type, "API" or "TheVault". */
export function sourceLabel(source: string): string {
  const [kind, ...rest] = source.split(":");
  const name = rest.join(":");
  switch (kind) {
    case "task":
      return name || "Scheduled script";
    case "handler":
      return name ? `${name.split(".")[0]} handler` : "Handler";
    case "token":
      return name || "API";
    case "script":
      return name || "Script";
    case "api":
      return "API";
    case "system":
      return "TheVault";
    default:
      return source;
  }
}

/** Where clicking a notification goes: its note, else its page, else nowhere. */
export function targetOf(n: Pick<AppNotification, "note" | "url">): string | null {
  if (n.note) return `/notes/${encodeURIComponent(n.note)}`;
  if (n.url && n.url.startsWith("/") && !n.url.startsWith("//")) return n.url;
  return null;
}

/** The panel's list, newest first, and the cursor of the page after it. */
export interface ListState {
  items: AppNotification[];
  next: string | null;
}

/** `state` with a later page added at its end, skipping anything already listed. */
export function appendPage(state: ListState, page: ListState): ListState {
  const seen = new Set(state.items.map((n) => n.id));
  return { items: [...state.items, ...page.items.filter((n) => !seen.has(n.id))], next: page.next };
}

/**
 * `state` with a fresh first page merged in: new notifications go on top, listed ones take their
 * fresh form, and the pages loaded beyond the first stay.
 */
export function mergeFirstPage(state: ListState, page: ListState): ListState {
  if (!state.items.length) return page;
  const fresh = new Map(page.items.map((n) => [n.id, n]));
  const known = new Set(state.items.map((n) => n.id));
  const added = page.items.filter((n) => !known.has(n.id));
  const kept = state.items.map((n) => fresh.get(n.id) ?? n);
  return { items: [...added, ...kept], next: state.next };
}

export function markRead(state: ListState, id: string, at: number): ListState {
  return { ...state, items: state.items.map((n) => (n.id === id && n.read === null ? { ...n, read: at } : n)) };
}

export function markAllRead(state: ListState, at: number): ListState {
  return { ...state, items: state.items.map((n) => (n.read === null ? { ...n, read: at } : n)) };
}

export function removeItem(state: ListState, id: string): ListState {
  return { ...state, items: state.items.filter((n) => n.id !== id) };
}

export function clearRead(state: ListState): ListState {
  return { ...state, items: state.items.filter((n) => n.read === null) };
}

/**
 * What to announce after a poll: the newest notification when it is new since the last poll and
 * unread, as "New notification: <title>", or a count when several came at once.
 */
export function announcement(prev: { newest: string | null; unread: number } | null, page: { items: AppNotification[]; unread: number }): string {
  const newest = page.items[0];
  if (!prev || !newest || newest.read !== null || newest.id === prev.newest) return "";
  if (prev.newest !== null && newest.id < prev.newest) return "";
  const more = page.unread - prev.unread;
  if (more > 1) return `${more} new notifications. Latest: ${newest.title}`;
  return `New notification: ${newest.title}`;
}
