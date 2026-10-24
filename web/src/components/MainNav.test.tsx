import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

// The sidebar's Outbox link (outbox X7): under Inbox, with a badge counting the items in Review;
// and the Log under Outbox (activity Y4).

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
  it("put Outbox under Inbox, counting the items waiting for review", async () => {
    query.mockImplementation((q: string) => Promise.resolve({ rows: [], next: null, total: q.startsWith("from outbox") ? 3 : 0 }));
    render(() => (
      <ul>
        <MainNavLinks />
      </ul>
    ));
    expect(screen.getAllByRole("link").map((a) => a.getAttribute("href"))).toEqual(["/daily", "/search", "/inbox", "/outbox", "/log"]);
    expect(link(/^Outbox/).querySelector(".nav-badge")).toBeNull();
    await refreshOutboxCount();
    await waitFor(() => expect(link(/^Outbox/).querySelector(".nav-badge")?.textContent).toBe("3"));
    expect(query).toHaveBeenCalledWith('from outbox where status == "review"', null, 1);
    expect(link(/^Outbox/).querySelector(".nav-badge")?.getAttribute("title")).toBe("3 waiting");
  });

  it("put the Log under Outbox, with its scroll icon and no badge", async () => {
    query.mockResolvedValue({ rows: [], next: null, total: 0 });
    render(() => (
      <ul>
        <MainNavLinks />
      </ul>
    ));
    const log = link(/^Log$/);
    expect(log.getAttribute("href")).toBe("/log");
    expect(log.closest("li")?.previousElementSibling?.querySelector("a")?.getAttribute("href")).toBe("/outbox");
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
