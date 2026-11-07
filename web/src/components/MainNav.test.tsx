import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

// The sidebar's Outbox link (outbox X7, X14): under Inbox, with a badge counting the drafts agents
// made, which wait on the user; Dispatched under Outbox (TV-80); and the Log under it (activity Y4).

const query = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, query } };
});

const { MainNavLinks } = await import("./MainNav");
const { refreshOutboxCount } = await import("../outbox/store");

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const link = (name: RegExp) => screen.getByRole("link", { name });

describe("the main links", () => {
  it("put Outbox under Inbox, counting the agents' drafts", async () => {
    query.mockImplementation((q: string) => Promise.resolve({ rows: [], next: null, total: q.startsWith("from outbox") ? 3 : 0 }));
    render(() => (
      <ul>
        <MainNavLinks />
      </ul>
    ));
    expect(screen.getAllByRole("link").map((a) => a.getAttribute("href"))).toEqual(["/planner", "/search", "/inbox", "/outbox", "/dispatched", "/log"]);
    expect(link(/^Outbox/).querySelector(".nav-badge")).toBeNull();
    await refreshOutboxCount();
    await waitFor(() => expect(link(/^Outbox/).querySelector(".nav-badge")?.textContent).toBe("3"));
    expect(query).toHaveBeenCalledWith('from outbox where status == "draft" && drafted_by != "user"', null, 1);
    expect(link(/^Outbox/).querySelector(".nav-badge")?.getAttribute("title")).toBe("3 waiting");
  });

  it("put Dispatched under Outbox, with its robot icon and no badge", async () => {
    query.mockResolvedValue({ rows: [], next: null, total: 0 });
    render(() => (
      <ul>
        <MainNavLinks />
      </ul>
    ));
    const sent = link(/^Dispatched$/);
    expect(sent.getAttribute("href")).toBe("/dispatched");
    expect(sent.closest("li")?.previousElementSibling?.querySelector("a")?.getAttribute("href")).toBe("/outbox");
    expect(sent.querySelector("svg.icon path")?.getAttribute("d")).toBe("M12 8V4H8");
    expect(sent.querySelector(".nav-badge")).toBeNull();
  });

  it("put the Log under Dispatched, with its scroll icon and no badge", async () => {
    query.mockResolvedValue({ rows: [], next: null, total: 0 });
    render(() => (
      <ul>
        <MainNavLinks />
      </ul>
    ));
    const log = link(/^Log$/);
    expect(log.getAttribute("href")).toBe("/log");
    expect(log.closest("li")?.previousElementSibling?.querySelector("a")?.getAttribute("href")).toBe("/dispatched");
    expect(log.querySelector("svg.icon path")?.getAttribute("d")).toBe("M15 12h-5");
    expect(log.querySelector(".nav-badge")).toBeNull();
  });

  it("shows no badge when nothing waits, or the count fails", async () => {
    query.mockResolvedValue({ rows: [], next: null, total: 0 });
    render(() => (
      <ul>
        <MainNavLinks />
      </ul>
    ));
    await refreshOutboxCount();
    expect(link(/^Outbox/).querySelector(".nav-badge")).toBeNull();
    query.mockRejectedValue(new Error("no type"));
    await refreshOutboxCount();
    expect(link(/^Outbox/).querySelector(".nav-badge")).toBeNull();
  });
});
