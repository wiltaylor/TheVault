// The Search page's selection and robots (TV-79): each result has a checkbox beside its link,
// "Select all on this page" selects the loaded results, the bar sends them, and each result shows
// the robots of its threads from one GET /api/dispatched for the loaded results.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DispatchedThread, Row } from "../api";

const { dispatched, query } = vi.hoisted(() => ({ dispatched: vi.fn(), query: vi.fn() }));
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, dispatched, query } };
});
vi.mock("@solidjs/router", () => ({
  useLocation: () => ({ search: "" }),
  useNavigate: () => vi.fn(),
  A: (p: { href: string; class?: string; children?: unknown; "data-testid"?: string }) => (
    <a href={p.href} class={p.class} data-testid={p["data-testid"]}>
      {p.children as never}
    </a>
  ),
}));

const { Search } = await import("./Search");
const { resetDispatched } = await import("../dispatch");

const rows: Row[] = [
  { id: "01JN1", type: "note", title: "Roof quote", fields: { updated: "2026-11-01T00:00:00Z" } },
  { id: "01JN2", type: "note", title: "Tiles", fields: { updated: "2026-11-02T00:00:00Z" } },
];

const thread: DispatchedThread = {
  id: "01JT",
  agent: "clerk",
  title: "Roof: file",
  origin: "mention",
  notes: ["01JN2"],
  state: "active",
  pending: 0,
  question: null,
  last_activity: "2026-11-07T05:00:00Z",
  running: true,
  turn: null,
  started_ms: null,
};

beforeEach(() => {
  resetDispatched();
  query.mockResolvedValue({ rows, next: null, total: rows.length });
  dispatched.mockResolvedValue({ threads: [thread] });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const resultOf = (title: string) => screen.getByText(title).closest(".search-row-line") as HTMLElement;

describe("the Search page", () => {
  it("shows each result's robots from one read, beside a checkbox outside the link", async () => {
    render(() => <Search />);
    await screen.findByText("Tiles");
    await waitFor(() => expect(within(resultOf("Tiles")).getByTestId("agent-robot")).toHaveAttribute("data-state", "active"));
    expect(dispatched).toHaveBeenCalledTimes(1);
    expect(dispatched).toHaveBeenCalledWith({ notes: ["01JN1", "01JN2"] });
    expect(within(resultOf("Roof quote")).queryByTestId("agent-robot")).toBeNull();
    // No link inside a link: the checkbox and the robot sit beside the result's link.
    const link = within(resultOf("Tiles")).getByTestId("note-row");
    expect(link.querySelector("a, input")).toBeNull();
  });

  it("selects all loaded results", async () => {
    render(() => <Search />);
    await screen.findByText("Tiles");
    fireEvent.click(screen.getByRole("checkbox", { name: "Select all on this page" }));
    expect(screen.getByRole("button", { name: "Send 2 notes to agent" })).toBeEnabled();
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Roof quote" }));
    expect(screen.getByRole("button", { name: "Send 1 note to agent" })).toBeEnabled();
  });
});
