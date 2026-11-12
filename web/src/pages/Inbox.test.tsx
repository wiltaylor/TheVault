// The Inbox's selection and robots (TV-79): every row has a checkbox, "Select all on this page"
// selects the rows shown, the bar sends them to an agent in one request and clears the selection,
// and each row shows the robots of its threads from one GET /api/dispatched for the page.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentSummary, DispatchedThread, Row } from "../api";
import type { Item } from "../inbox/sources";

const { dispatched, dispatch, agents, loadInbox } = vi.hoisted(() => ({ dispatched: vi.fn(), dispatch: vi.fn(), agents: vi.fn(), loadInbox: vi.fn() }));
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, dispatched, dispatch, agents, heldAgentNotes: vi.fn().mockResolvedValue({ notes: [], total: 0 }), app: vi.fn().mockResolvedValue(null), note: vi.fn().mockRejectedValue(new Error("no")) } };
});
vi.mock("../inbox/load", () => ({ loadInbox, countInbox: vi.fn().mockResolvedValue(0) }));
vi.mock("@solidjs/router", () => ({ useNavigate: () => vi.fn() }));

const { Inbox } = await import("./Inbox");
const { api } = await import("../api");
const { Toasts, dismissToast } = await import("../components/Toasts");
const { refreshAgents } = await import("../agents");
const { resetDispatched } = await import("../dispatch");
const { SOURCES } = await import("../inbox/sources");

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
  document.querySelectorAll<HTMLElement>("[data-toast]").forEach((el) => dismissToast(Number(el.dataset.toast)));
  cleanup();
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

const rowOf = (title: string) => screen.getByRole("link", { name: title }).closest("li")!;

describe("the Inbox", () => {
  it("keeps a convert picker and its input across refreshed and reordered rows", async () => {
    const capture = { ...items[0], source: SOURCES.find((s) => s.key === "captures")! };
    loadInbox.mockResolvedValueOnce({ items: [capture], total: 1 });
    render(() => <Inbox />);
    await screen.findByRole("link", { name: "Roof quote" });
    const originalRow = rowOf("Roof quote");
    fireEvent.click(within(originalRow).getByRole("button", { name: "Convert to…" }));
    const input = await screen.findByRole("combobox");
    fireEvent.input(input, { target: { value: "Ta" } });
    loadInbox.mockResolvedValueOnce({ items: [items[1], {
      ...capture, row: { ...capture.row, title: "Updated capture" },
    }], total: 2 });
    fireEvent.click(screen.getByRole("button", { name: "Inbox", exact: true }));
    await screen.findByRole("link", { name: "Updated capture" });
    expect(rowOf("Updated capture")).toBe(originalRow);
    expect(screen.getByRole("combobox")).toBe(input);
    expect(input).toHaveValue("Ta");
    fireEvent.input(input, { target: { value: "Task" } });
    expect(input).toHaveValue("Task");
    expect(within(originalRow).getByRole("button", { name: "Convert to…" })).toHaveAttribute("aria-expanded", "true");
  });

  it("shows held counts and separates scams from other holds", async () => {
    vi.mocked(api.heldAgentNotes).mockResolvedValueOnce({ notes: [
      { id: "scam", type: "email", created: null, agent_safety: { status: "held", reason: "scam", fingerprint: "a" } },
      { id: "other", type: "email", created: null, agent_safety: { status: "held", reason: "prompt_injection", fingerprint: "b" } },
    ], total: 2 });
    render(() => <Inbox />);
    fireEvent.click(await screen.findByText("Possible scams (1)"));
    await screen.findByText("Reason: scam");
    expect(screen.queryByText("Reason: prompt_injection")).toBeNull();
    expect(screen.queryByRole("link", { name: "Roof quote" })).toBeNull();
    fireEvent.click(screen.getByText("Other held (1)"));
    await screen.findByText("Reason: prompt_injection");
    expect(screen.queryByText("Reason: scam")).toBeNull();
  });
  it("sends a single card through its robot action", async () => {
    dispatch.mockResolvedValue({ thread: { id: "01NEW" } });
    render(() => <Inbox />);
    await screen.findByRole("link", { name: "Roof quote" });
    const button = within(rowOf("Roof quote")).getByRole("button", { name: "Send to agent" });
    expect(button.querySelector("svg.robot-glyph")).not.toBeNull();
    fireEvent.click(button);
    fireEvent.input(screen.getByRole("textbox"), { target: { value: "File it" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("status");
    expect(dispatch).toHaveBeenCalledWith("clerk", ["01JN1"], "File it");
  });

  it.each(["archive", "delete"] as const)("bulk %s acts on two selected cards only", async (action) => {
    loadInbox.mockResolvedValue({ items: items.map((i) => ({ ...i, source: SOURCES.find((s) => s.key === "messages")! })), total: 3 });
    vi.spyOn(api, "note").mockImplementation(async (id) => ({ id, type: "message", wcl: "", version: "v1", note: { fields: {}, blocks: [] } }));
    const archive = vi.spyOn(api, "archiveNote").mockResolvedValue({ version: "v2" } as Awaited<ReturnType<typeof api.note>>);
    const remove = vi.spyOn(api, "deleteNote").mockResolvedValue(undefined);
    const restore = vi.spyOn(api, "restoreFromTrash").mockResolvedValue({} as Awaited<ReturnType<typeof api.note>>);
    const unarchive = vi.spyOn(api, "unarchiveNote").mockResolvedValue({} as Awaited<ReturnType<typeof api.note>>);
    render(() => <><Inbox /><Toasts /></>);
    await screen.findByRole("link", { name: "Roof quote" });
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Roof quote" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Gutter invoice" }));
    fireEvent.click(screen.getByRole("button", { name: action === "archive" ? "Archive selected" : "Delete selected" }));
    const called = action === "archive" ? archive : remove;
    await waitFor(() => expect(called).toHaveBeenCalledTimes(2));
    expect(called).toHaveBeenNthCalledWith(1, "01JN1", "v1");
    expect(called).toHaveBeenNthCalledWith(2, "01JN2", "v1");
    await waitFor(() => expect(screen.queryByRole("link", { name: "Roof quote" })).toBeNull());
    expect(screen.getByRole("link", { name: "Tiles" })).toBeInTheDocument();
    expect(await screen.findByText(action === "archive" ? "2 notes archived" : "2 notes moved to the trash")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    const undone = action === "archive" ? unarchive : restore;
    await waitFor(() => expect(undone).toHaveBeenCalledTimes(2));
    if (action === "archive") {
      expect(unarchive).toHaveBeenNthCalledWith(1, "01JN1", "v2");
      expect(unarchive).toHaveBeenNthCalledWith(2, "01JN2", "v2");
    } else {
      expect(restore).toHaveBeenNthCalledWith(1, "01JN1");
      expect(restore).toHaveBeenNthCalledWith(2, "01JN2");
    }
    expect(await screen.findByRole("link", { name: "Roof quote" })).toBeInTheDocument();
    expect(await screen.findByRole("link", { name: "Gutter invoice" })).toBeInTheDocument();
  });

  it("reports partial bulk failure and lets Undo restore only completed notes", async () => {
    loadInbox.mockResolvedValue({ items: items.map((i) => ({ ...i, source: SOURCES.find((s) => s.key === "messages")! })), total: 3 });
    vi.spyOn(api, "note").mockImplementation(async (id) => ({ id, type: "message", wcl: "", version: "v1", note: { fields: {}, blocks: [] } }));
    vi.spyOn(api, "deleteNote").mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("conflict"));
    const restore = vi.spyOn(api, "restoreFromTrash").mockResolvedValue({} as Awaited<ReturnType<typeof api.note>>);
    render(() => <><Inbox /><Toasts /></>);
    await screen.findByRole("link", { name: "Roof quote" });
    fireEvent.click(screen.getByRole("checkbox", { name: "Select all on this page" }));
    fireEvent.click(screen.getByRole("button", { name: "Delete selected" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("1 note moved to the trash before the error: conflict");
    expect(screen.getByRole("checkbox", { name: "Select Gutter invoice" })).toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(restore).toHaveBeenCalledTimes(1));
    expect(restore).toHaveBeenCalledWith("01JN1");
    expect(await screen.findByRole("link", { name: "Roof quote" })).toBeInTheDocument();
  });

  it("keeps bulk delete unavailable when mail is selected", async () => {
    render(() => <Inbox />);
    await screen.findByRole("link", { name: "Roof quote" });
    fireEvent.click(screen.getByRole("checkbox", { name: "Select all on this page" }));
    expect(screen.getByRole("button", { name: "Delete selected" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Archive selected" })).toBeEnabled();
  });
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
