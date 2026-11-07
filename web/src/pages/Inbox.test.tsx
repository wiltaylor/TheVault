// The Inbox's selection and robots (TV-79): every row has a checkbox, "Select all on this page"
// selects the rows shown, the bar sends them to an agent in one request and clears the selection,
// and each row shows the robots of its threads from one GET /api/dispatched for the page.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentSummary, DispatchedThread, Row } from "../api";
import { SOURCES, type Item } from "../inbox/sources";

const dispatched = vi.fn();
const dispatch = vi.fn();
const agents = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, dispatched, dispatch, agents, app: vi.fn().mockResolvedValue(null), note: vi.fn().mockRejectedValue(new Error("no")) } };
});
const loadInbox = vi.fn();
vi.mock("../inbox/load", () => ({ loadInbox, countInbox: vi.fn().mockResolvedValue(0) }));
vi.mock("@solidjs/router", () => ({ useNavigate: () => vi.fn() }));

const { Inbox } = await import("./Inbox");
const { refreshAgents } = await import("../agents");
const { resetDispatched } = await import("../dispatch");

const MAIL = SOURCES.find((s) => s.key === "mail")!;
const row = (id: string, title: string): Row => ({ id, type: "email", title, fields: { from_raw: "a@b.c" } });
const items: Item[] = [row("01JN1", "Roof quote"), row("01JN2", "Gutter invoice"), row("01JN3", "Tiles")].map((r, i) => ({
  source: MAIL,
  row: r,
  time: 1_700_000_000_000 - i,
}));

function thread(over: Partial<DispatchedThread>): DispatchedThread {
  return {
    id: "01JT",
    agent: "clerk",
    title: "Roof: file",
    origin: "dispatch",
    notes: ["01JN1"],
    state: "needs_you",
    pending: 1,
    question: null,
    last_activity: "2026-11-07T05:00:00Z",
    running: false,
    turn: null,
    started_ms: null,
    ...over,
  };
}

beforeEach(async () => {
  resetDispatched();
  loadInbox.mockResolvedValue({ items, total: items.length });
  dispatched.mockResolvedValue({ threads: [thread({})] });
  agents.mockResolvedValue({ agents: [{ name: "clerk", label: "Clerk", icon: null, built_in: false, enabled: true }] as AgentSummary[], tools: [] });
  await refreshAgents();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const rowOf = (title: string) => screen.getByRole("link", { name: title }).closest("li")!;

describe("the Inbox", () => {
  it("shows each row's robots from one read for the page", async () => {
    render(() => <Inbox />);
    await screen.findByRole("link", { name: "Roof quote" });
    await waitFor(() => expect(within(rowOf("Roof quote")).getByTestId("agent-robot")).toHaveAttribute("data-state", "needs_you"));
    expect(dispatched).toHaveBeenCalledTimes(1);
    expect(dispatched).toHaveBeenCalledWith({ notes: ["01JN1", "01JN2", "01JN3"] });
    expect(within(rowOf("Tiles")).queryByTestId("agent-robot")).toBeNull();
  });

  it("selects all rows on the page and sends them in one request", async () => {
    dispatch.mockResolvedValue({ thread: { id: "01NEW", agent: "clerk", title: "t", origin: "dispatch" } });
    render(() => <Inbox />);
    await screen.findByRole("link", { name: "Roof quote" });
    fireEvent.click(screen.getByRole("checkbox", { name: "Select all on this page" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Gutter invoice" }));
    fireEvent.click(screen.getByRole("button", { name: "Send 2 notes to agent" }));
    fireEvent.input(screen.getByRole("textbox"), { target: { value: "File the quotes" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Sent to Clerk.");
    expect(dispatch).toHaveBeenCalledTimes(1);
    expect(dispatch).toHaveBeenCalledWith("clerk", ["01JN1", "01JN3"], "File the quotes");
    expect(screen.queryByRole("button", { name: /^Send \d+ notes? to agent$/ })).toBeNull();
    // The rows' robots are read again so the new thread shows.
    await waitFor(() => expect(dispatched).toHaveBeenCalledTimes(2));
  });
});
