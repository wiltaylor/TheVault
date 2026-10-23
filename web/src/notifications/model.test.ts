import { describe, expect, it } from "vitest";
import type { AppNotification } from "../api";
import {
  type ListState,
  announcement,
  appendPage,
  applyEvent,
  badgeText,
  bellLabel,
  clearRead,
  markAllRead,
  markRead,
  mergeFirstPage,
  relativeTime,
  removeItem,
  sourceLabel,
  targetOf,
} from "./model";

function n(id: string, extra: Partial<AppNotification> = {}): AppNotification {
  return {
    id,
    created: 0,
    title: `title ${id}`,
    body: null,
    note: null,
    note_type: null,
    note_title: null,
    url: null,
    source: "api",
    read: null,
    ...extra,
  };
}

describe("badgeText", () => {
  it("hides at zero and caps at 99+", () => {
    expect(badgeText(0)).toBe("");
    expect(badgeText(-3)).toBe("");
    expect(badgeText(1)).toBe("1");
    expect(badgeText(99)).toBe("99");
    expect(badgeText(100)).toBe("99+");
    expect(badgeText(4000)).toBe("99+");
  });

  it("names the count in the bell's label", () => {
    expect(bellLabel(0)).toBe("Notifications");
    expect(bellLabel(3)).toBe("Notifications, 3 unread");
  });
});

describe("relativeTime", () => {
  const now = new Date(2026, 9, 15, 12, 0, 0).getTime();
  const MIN = 60_000;
  it("says how long ago, briefly", () => {
    expect(relativeTime(now - 10_000, now)).toBe("just now");
    expect(relativeTime(now + 5_000, now)).toBe("just now");
    expect(relativeTime(now - 5 * MIN, now)).toBe("5 min ago");
    expect(relativeTime(now - 59 * MIN, now)).toBe("59 min ago");
    expect(relativeTime(now - 60 * MIN, now)).toBe("1 h ago");
    expect(relativeTime(now - 23 * 60 * MIN, now)).toBe("23 h ago");
    expect(relativeTime(now - 25 * 60 * MIN, now)).toBe("yesterday");
    expect(relativeTime(now - 4 * 24 * 60 * MIN, now)).toBe("4 d ago");
  });

  it("gives the date after a week, with the year when it is another", () => {
    expect(relativeTime(new Date(2026, 2, 3, 9).getTime(), now)).toBe("3 Mar");
    expect(relativeTime(new Date(2025, 11, 24, 9).getTime(), now)).toBe("24 Dec 2025");
  });
});

describe("sourceLabel", () => {
  it("names who a notification is from", () => {
    expect(sourceLabel("task:digest")).toBe("digest");
    expect(sourceLabel("handler:invoice.after_create")).toBe("invoice handler");
    expect(sourceLabel("token:claude")).toBe("claude");
    expect(sourceLabel("api")).toBe("API");
    expect(sourceLabel("system")).toBe("TheVault");
    expect(sourceLabel("other")).toBe("other");
  });
});

describe("targetOf", () => {
  it("opens the note, else the page", () => {
    expect(targetOf({ note: "01AB", url: "/settings" })).toBe("/notes/01AB");
    expect(targetOf({ note: null, url: "/settings/scheduled/digest" })).toBe("/settings/scheduled/digest");
    expect(targetOf({ note: null, url: "//evil.example" })).toBeNull();
    expect(targetOf({ note: null, url: null })).toBeNull();
  });
});

describe("the panel's list", () => {
  it("appends later pages without repeats", () => {
    const s = appendPage({ items: [n("3"), n("2")], next: "2" }, { items: [n("2"), n("1")], next: null });
    expect(s.items.map((x) => x.id)).toEqual(["3", "2", "1"]);
    expect(s.next).toBeNull();
  });

  it("puts new notifications from a fresh first page on top and keeps loaded pages", () => {
    const s = mergeFirstPage(
      { items: [n("3"), n("2"), n("1")], next: "1" },
      { items: [n("5"), n("4"), n("3", { read: 7 })], next: "3" },
    );
    expect(s.items.map((x) => x.id)).toEqual(["5", "4", "3", "2", "1"]);
    expect(s.items[2]!.read).toBe(7);
    expect(s.next).toBe("1");
    expect(mergeFirstPage({ items: [], next: null }, { items: [n("1")], next: null }).items).toHaveLength(1);
  });

  it("marks read, dismisses and clears", () => {
    let s: ListState = { items: [n("3"), n("2", { read: 1 }), n("1")], next: null };
    s = markRead(s, "3", 9);
    expect(s.items.map((x) => x.read)).toEqual([9, 1, null]);
    expect(markRead(s, "2", 9).items[1]!.read).toBe(1);
    s = removeItem(s, "1");
    expect(s.items.map((x) => x.id)).toEqual(["3", "2"]);
    expect(clearRead(s).items).toEqual([]);
    const all = markAllRead({ items: [n("2"), n("1", { read: 4 })], next: null }, 8);
    expect(all.items.map((x) => x.read)).toEqual([8, 4]);
  });
});

describe("announcement", () => {
  it("announces only a notification new since the last poll", () => {
    const page = (items: AppNotification[], unread: number) => ({ items, unread });
    expect(announcement(null, page([n("2")], 1))).toBe("");
    expect(announcement({ newest: "1", unread: 0 }, page([n("2")], 1))).toBe("New notification: title 2");
    expect(announcement({ newest: "2", unread: 1 }, page([n("2")], 1))).toBe("");
    expect(announcement({ newest: null, unread: 0 }, page([n("4")], 3))).toBe("3 new notifications. Latest: title 4");
    expect(announcement({ newest: "1", unread: 0 }, page([n("2", { read: 3 })], 0))).toBe("");
    expect(announcement({ newest: "3", unread: 0 }, page([n("2")], 1))).toBe("");
  });
});

describe("applyEvent", () => {
  const state = (): ListState => ({ items: [n("C"), n("B", { read: 5 }), n("A")], next: "A" });
  const ids = (s: ListState) => s.items.map((x) => `${x.id}${x.read === null ? "" : "*"}`);

  it("leaves the list alone for a new notification, which the caller fetches", () => {
    const s = state();
    expect(applyEvent(s, { name: "notification.created", data: { id: "D", unread: 3 } }, 9)).toBe(s);
    // A changed one's new text is fetched too.
    expect(applyEvent(s, { name: "notification.updated", data: { id: "C", unread: 3 } }, 9)).toBe(s);
  });

  it("marks one read, or all when the event names none", () => {
    expect(ids(applyEvent(state(), { name: "notification.read", data: { id: "A", unread: 1 } }, 9))).toEqual(["C", "B*", "A*"]);
    expect(ids(applyEvent(state(), { name: "notification.read", data: { id: null, unread: 0 } }, 9))).toEqual(["C*", "B*", "A*"]);
  });

  it("removes a dismissed one and clears the read ones", () => {
    expect(ids(applyEvent(state(), { name: "notification.dismissed", data: { id: "C", unread: 1 } }, 9))).toEqual(["B*", "A"]);
    expect(ids(applyEvent(state(), { name: "notification.cleared", data: { cleared: 1, unread: 2 } }, 9))).toEqual(["C", "A"]);
  });
});
