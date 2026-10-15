// The notifications the bell shows: the unread count, polled every 30 seconds and whenever the
// window comes back, and the panel's list once it has been opened. The server has no push
// channel, so polling is how a notification made elsewhere reaches the badge.

import { createSignal } from "solid-js";
import { api } from "../api";
import {
  type ListState,
  announcement,
  appendPage,
  clearRead,
  markAllRead,
  markRead,
  mergeFirstPage,
  removeItem,
} from "./model";

/** How often the unread count is asked for, in milliseconds. */
export const POLL_MS = 30_000;

/** How many notifications the panel loads at a time. */
export const PAGE_SIZE = 20;

/**
 * The poll interval. Test-only: an end-to-end test sets `window.__THEVAULT_TEST_NOTIFICATION_POLL_MS__`
 * before the app loads to poll faster. Nothing in the app sets it.
 */
export function pollInterval(): number {
  const v = (window as { __THEVAULT_TEST_NOTIFICATION_POLL_MS__?: unknown }).__THEVAULT_TEST_NOTIFICATION_POLL_MS__;
  return typeof v === "number" && v > 0 ? v : POLL_MS;
}

const [unread, setUnread] = createSignal(0);
const [list, setList] = createSignal<ListState>({ items: [], next: null });
const [loaded, setLoaded] = createSignal(false);
const [loading, setLoading] = createSignal(false);
const [failed, setFailed] = createSignal(false);
const [open, setOpen] = createSignal(false);
const [said, setSaid] = createSignal("");

/** Reactive: how many notifications are unread. */
export const unreadCount = unread;
/** Reactive: the panel's list and the cursor of its next page. */
export const notificationList = list;
/** Reactive: whether the panel has its first page. */
export const listLoaded = loaded;
/** Reactive: whether a page is on its way. */
export const listLoading = loading;
/** Reactive: whether the last load failed. */
export const listFailed = failed;
/** Reactive: whether the panel shows. */
export const panelOpen = open;
/** Reactive: the live region's text, announcing a notification that just came. */
export const announced = said;

let polls = 0;
let loads = 0;
let lastSeen: { newest: string | null; unread: number } | null = null;

/** Ask for the unread count and the newest notification; merge them into the list while it shows. */
export async function refreshNotifications(): Promise<void> {
  const mine = ++polls;
  try {
    const page = await api.notifications({ limit: open() ? PAGE_SIZE : 1 });
    if (mine !== polls) return;
    setUnread(page.unread);
    const say = announcement(lastSeen, page);
    if (say) setSaid(say);
    lastSeen = { newest: page.items[0]?.id ?? null, unread: page.unread };
    if (open() && loaded()) setList((s) => mergeFirstPage(s, page));
  } catch {
    // A failed poll leaves the badge as it was; the next one tries again.
  }
}

/** Poll until the returned function is called: every [`pollInterval`] while the page shows, and on focus. */
export function startPolling(): () => void {
  void refreshNotifications();
  const timer = setInterval(() => {
    if (document.visibilityState !== "hidden") void refreshNotifications();
  }, pollInterval());
  const onFocus = () => void refreshNotifications();
  const onVisible = () => {
    if (document.visibilityState === "visible") void refreshNotifications();
  };
  window.addEventListener("focus", onFocus);
  document.addEventListener("visibilitychange", onVisible);
  return () => {
    clearInterval(timer);
    window.removeEventListener("focus", onFocus);
    document.removeEventListener("visibilitychange", onVisible);
  };
}

async function loadFirst() {
  const mine = ++loads;
  setLoading(true);
  setFailed(false);
  try {
    const page = await api.notifications({ limit: PAGE_SIZE });
    if (mine !== loads) return;
    setUnread(page.unread);
    lastSeen = { newest: page.items[0]?.id ?? null, unread: page.unread };
    setList({ items: page.items, next: page.next });
    setLoaded(true);
  } catch {
    if (mine === loads) setFailed(true);
  } finally {
    if (mine === loads) setLoading(false);
  }
}

export function openPanel() {
  if (open()) return;
  setOpen(true);
  void loadFirst();
}

export function closePanel() {
  setOpen(false);
}

export function togglePanel() {
  if (open()) closePanel();
  else openPanel();
}

/** Load the page after the last one shown. */
export async function loadMore(): Promise<void> {
  const cursor = list().next;
  if (!cursor || loading()) return;
  setLoading(true);
  try {
    const page = await api.notifications({ limit: PAGE_SIZE, cursor });
    setUnread(page.unread);
    setList((s) => appendPage(s, page));
  } catch {
    setFailed(true);
  } finally {
    setLoading(false);
  }
}

/** Mark one read, at once on screen and then on the server. */
export async function readNotification(id: string): Promise<void> {
  const n = list().items.find((x) => x.id === id);
  if (n && n.read !== null) return;
  setList((s) => markRead(s, id, Date.now()));
  setUnread((u) => Math.max(0, u - 1));
  try {
    setUnread((await api.readNotification(id)).unread);
  } catch {
    void refreshNotifications();
  }
}

export async function readAll(): Promise<void> {
  setList((s) => markAllRead(s, Date.now()));
  setUnread(0);
  try {
    await api.readAllNotifications();
  } catch {
    void refreshNotifications();
  }
}

export async function dismiss(id: string): Promise<void> {
  const n = list().items.find((x) => x.id === id);
  setList((s) => removeItem(s, id));
  if (n && n.read === null) setUnread((u) => Math.max(0, u - 1));
  try {
    setUnread((await api.dismissNotification(id)).unread);
  } catch {
    void refreshNotifications();
  }
}

export async function clearReadItems(): Promise<void> {
  setList((s) => clearRead(s));
  try {
    setUnread((await api.clearReadNotifications()).unread);
  } catch {
    void refreshNotifications();
  }
}
