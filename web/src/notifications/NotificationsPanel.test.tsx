import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppNotification } from "../api";

function n(id: string, extra: Partial<AppNotification> = {}): AppNotification {
  return {
    id,
    created: Date.now() - 5 * 60_000,
    title: `Title ${id}`,
    body: null,
    note: null,
    note_type: null,
    note_title: null,
    url: null,
    source: "task:digest",
    read: null,
    ...extra,
  };
}

let server: AppNotification[] = [];
const unread = () => server.filter((x) => x.read === null).length;
const notifications = vi.fn(async (opts: { limit?: number; cursor?: string | null } = {}) => {
  const from = opts.cursor ? server.findIndex((x) => x.id === opts.cursor) + 1 : 0;
  const limit = opts.limit ?? 50;
  const items = server.slice(from, from + limit);
  const next = from + limit < server.length ? items.at(-1)!.id : null;
  return { items, next, unread: unread() };
});
const readNotification = vi.fn(async (id: string) => {
  server = server.map((x) => (x.id === id ? { ...x, read: 1 } : x));
  return { ...server.find((x) => x.id === id)!, unread: unread() };
});
const readAllNotifications = vi.fn(async () => {
  server = server.map((x) => ({ ...x, read: x.read ?? 1 }));
  return { marked: 0, unread: 0 };
});
const dismissNotification = vi.fn(async (id: string) => {
  server = server.filter((x) => x.id !== id);
  return { id, deleted: true, unread: unread() };
});
const clearReadNotifications = vi.fn(async () => {
  server = server.filter((x) => x.read === null);
  return { cleared: 0, unread: unread() };
});
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    api: { ...real.api, notifications, readNotification, readAllNotifications, dismissNotification, clearReadNotifications },
  };
});

const { NotificationsBell } = await import("./NotificationsPanel");
const store = await import("./store");

afterEach(() => {
  store.closePanel();
  cleanup();
});

describe("NotificationsBell", () => {
  it("shows the unread count, opens the panel and acts on items", async () => {
    server = [n("05"), n("04", { body: "Line one\nline two" }), n("03", { read: 1 })];
    render(() => <NotificationsBell />);
    await store.refreshNotifications();
    const bell = screen.getByRole("button", { name: "Notifications, 2 unread" });
    expect(bell.textContent).toContain("2");
    expect(bell.getAttribute("aria-expanded")).toBe("false");

    fireEvent.click(bell);
    expect(bell.getAttribute("aria-expanded")).toBe("true");
    const dialog = await waitFor(() => screen.getByRole("dialog", { name: "Notifications" }));
    await waitFor(() => expect(dialog.querySelectorAll("li")).toHaveLength(3));
    expect(dialog.textContent).toContain("5 min ago");
    expect(dialog.textContent).toContain("digest");
    expect(dialog.querySelectorAll(".notif-item.unread")).toHaveLength(2);

    // An item without a note or page only marks itself read.
    fireEvent.click(screen.getByRole("button", { name: /^Unread:\s*Title 05/ }));
    await waitFor(() => expect(dialog.querySelectorAll(".notif-item.unread")).toHaveLength(1));
    expect(readNotification).toHaveBeenCalledWith("05");
    expect(screen.getByRole("button", { name: "Notifications, 1 unread" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Dismiss Title 04" }));
    await waitFor(() => expect(dialog.querySelectorAll("li")).toHaveLength(2));
    expect(screen.getByRole("button", { name: "Notifications" }).querySelector(".notif-badge")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Clear read" }));
    await waitFor(() => expect(screen.getByText("No notifications")).toBeTruthy());

    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Notifications" }));
  });

  it("pages with Load more and marks all read", async () => {
    server = Array.from({ length: 25 }, (_, i) => n(String(100 - i)));
    render(() => <NotificationsBell />);
    fireEvent.click(screen.getByRole("button", { name: /Notifications/ }));
    const dialog = await waitFor(() => screen.getByRole("dialog"));
    await waitFor(() => expect(dialog.querySelectorAll("li")).toHaveLength(store.PAGE_SIZE));
    fireEvent.click(screen.getByRole("button", { name: "Load more" }));
    await waitFor(() => expect(dialog.querySelectorAll("li")).toHaveLength(25));
    expect(screen.queryByRole("button", { name: "Load more" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Mark all read" }));
    await waitFor(() => expect(dialog.querySelectorAll(".notif-item.unread")).toHaveLength(0));
    expect(readAllNotifications).toHaveBeenCalled();
    expect((screen.getByRole("button", { name: "Mark all read" }) as HTMLButtonElement).disabled).toBe(true);

    // A click outside closes the panel.
    fireEvent.pointerDown(document.body);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("caps the badge at 99+", async () => {
    server = Array.from({ length: 120 }, (_, i) => n(String(1000 + i)));
    render(() => <NotificationsBell />);
    await store.refreshNotifications();
    expect(screen.getByRole("button", { name: "Notifications, 120 unread" }).textContent).toContain("99+");
  });
});
