// The agent screen's Chat tab (agents G5, G7, G8) and the AI panel's agent picker (agents G6):
// the thread list with its pending badges, New thread, a thread started by the first message
// with only that message sent, reopened threads with their pending cards, outcomes recorded in
// the thread, and the panel's conversations as threads of the chosen agent. The panel's thread
// picker, and a thread open in the panel and the Chat tab at once as one conversation (agents
// G18).

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentThread, StreamEvent } from "../api";
import { turnsOver } from "./fakeTurns";

type OnEvent = (e: StreamEvent) => void;

let live: { onEvent: OnEvent; body: Record<string, unknown>; end: () => void } | null = null;

const streamEvents = vi.fn(
  (_path: string, body: unknown, onEvent: OnEvent, signal?: AbortSignal) =>
    new Promise<void>((resolve, reject) => {
      live = { onEvent, body: body as Record<string, unknown>, end: resolve };
      signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
    }),
);
const agentThreads = vi.fn();
const agentThread = vi.fn();
const agentThreadStart = vi.fn();
const aiApply = vi.fn();
const aiDiscard = vi.fn();
const aiModels = vi.fn();
const aiProviders = vi.fn();
const agentsList = vi.fn();
const app = vi.fn();
const renameAgentThread = vi.fn();
const deleteAgentThread = vi.fn();
const archiveNote = vi.fn();
const unarchiveNote = vi.fn();
const aiTurnPoll = vi.fn();

const turns = turnsOver(streamEvents);

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    streamEvents,
    streamTurn: turns.streamTurn,
    api: { ...real.api, aiTurnStart: turns.aiTurnStart, aiTurnStop: turns.aiTurnStop, aiTurnPoll, agentThreads, agentThread, agentThreadStart, aiApply, aiDiscard, aiModels, aiProviders, agents: agentsList, app, renameAgentThread, deleteAgentThread, archiveNote, unarchiveNote },
  };
});
/** The app the page runs in, for the tests of the apps' polling transport (ai D23); none by default. */
const inApp = vi.hoisted(() => ({ now: null as null | { server_url: string } }));
vi.mock("../inApp", () => ({ appInfo: async () => inApp.now }));
const navigatePane = vi.fn();
vi.mock("../workspace/store", async (orig) => ({ ...(await orig<typeof import("../workspace/store")>()), navigatePane }));

const { AgentChat } = await import("./AgentChat");
const { ChatPanel, PANEL_KEY, forgetPanelChoice } = await import("./ChatPanel");
const { newConversation, resetThreads, setAgent, turnsFromThread } = await import("./store");
const { refreshAgents } = await import("../agents");
const { hasNewReply, liveTurn, resetRunning } = await import("./running");

const PROVIDERS = {
  default_provider: "anthropic",
  providers: {
    anthropic: { configured: true, model: "claude-opus-5" },
    openai: { configured: false, model: null },
    chatgpt: { configured: false, model: null },
  },
  confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true, agents: true },
};

const PROPOSAL = {
  id: "toolu_p",
  op: "update",
  note_id: "01JNOTE",
  title: "Groceries",
  base_version: "v1",
  before: { fields: { title: "Groceries" }, blocks: [] },
  after: { fields: { title: "Groceries and more" }, blocks: [] },
  ops: [{ op: "field", name: "title", before: "Groceries", after: "Groceries and more" }],
  agent: "triage",
  thread: "01JTHREAD",
};

/** A stored thread: one question, a step, an edit proposal still pending, and the answer. */
function thread(over: Partial<AgentThread> = {}): AgentThread {
  return {
    id: "01JTHREAD",
    title: "Groceries",
    agent: "triage",
    version: "t1",
    pending: 1,
    last_activity: "2026-10-23T04:00:00+00:00",
    messages: [
      { role: "user", content: [{ type: "text", text: "Tidy my list." }] },
      { role: "assistant", content: [{ type: "tool_call", id: "toolu_p", name: "propose_edit", input: {} }] },
      { role: "user", content: [{ type: "tool_result", id: "toolu_p", content: "{}", is_error: false }] },
      { role: "assistant", content: [{ type: "text", text: "Here is an edit." }] },
    ],
    steps: { toolu_p: { summary: { notes: [{ id: "01JNOTE", title: "Groceries" }] }, is_error: false } },
    proposals: [{ event: "proposal", data: PROPOSAL, status: "pending" }],
    ...over,
  };
}

beforeEach(() => {
  app.mockResolvedValue(null);
  aiProviders.mockResolvedValue(PROVIDERS);
  aiModels.mockResolvedValue({ provider: "anthropic", model: "claude-opus-5", models: [{ id: "claude-opus-5", display_name: "Claude Opus 5" }] });
  agentThreads.mockResolvedValue({
    threads: [
      { id: "01JTHREAD", title: "Groceries", pending: 1, last_activity: "2026-10-23T04:00:00+00:00" },
      { id: "01JOLDER", title: "Roof", pending: 0, last_activity: "2026-10-22T04:00:00+00:00" },
    ],
  });
  agentThread.mockResolvedValue(thread());
  agentThreadStart.mockResolvedValue({ id: "01JNEW", title: "New thread", agent: "triage", version: "n1", pending: 0 });
  aiApply.mockResolvedValue({ op: "update", note_id: "01JNOTE", version: "v2" });
  aiDiscard.mockResolvedValue({ thread: "01JTHREAD", pending: 0 });
  agentsList.mockResolvedValue({ agents: [], tools: [] });
  renameAgentThread.mockResolvedValue(thread({ title: "Shopping" }));
  deleteAgentThread.mockResolvedValue({ id: "01JTHREAD", deleted: true });
  archiveNote.mockResolvedValue({});
  unarchiveNote.mockResolvedValue({});
});

afterEach(async () => {
  newConversation();
  setAgent("assistant");
  live = null;
  cleanup();
  resetThreads();
  vi.clearAllMocks();
  agentsList.mockResolvedValue({ agents: [], tools: [] });
  await refreshAgents();
});

async function send(text: string) {
  fireEvent.input(screen.getByLabelText("Ask AI"), { target: { value: text } });
  fireEvent.click(screen.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
}

describe("the Chat tab", () => {
  it("lists the agent's threads, newest first, with a badge on the one with pending proposals", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = within(screen.getByRole("complementary", { name: "Threads" }));
    await waitFor(() => expect(list.getAllByRole("button", { name: /^(Groceries|Roof)/ })).toHaveLength(2));
    expect(agentThreads).toHaveBeenCalledWith("triage");
    const items = list.getAllByRole("listitem");
    expect(items[0]).toHaveTextContent("Groceries");
    expect(items[1]).toHaveTextContent("Roof");
    expect(within(items[0]!).getByLabelText("1 proposal waits for you")).toHaveTextContent("1");
    expect(within(items[1]!).queryByLabelText(/wait for you|waits for you/)).toBeNull();
  });

  it("starts a thread with the first message and sends only that message", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    await waitFor(() => expect(agentThreads).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("button", { name: "New thread" }));
    await send("File my email.");
    expect(agentThreadStart).toHaveBeenCalledWith("triage");
    expect(live!.body).toMatchObject({ agent: "triage", thread: "01JNEW" });
    expect(live!.body.messages).toEqual([{ role: "user", content: [{ type: "text", text: "File my email." }] }]);
    const calls = agentThreads.mock.calls.length;
    live!.onEvent({ event: "text", data: { delta: "Done." } });
    live!.onEvent({ event: "done", data: { stop: "end_turn", messages: [] } });
    live!.end();
    await waitFor(() => expect(agentThreads.mock.calls.length).toBeGreaterThan(calls));
    // The next message stays on the thread.
    live = null;
    await send("And the rest.");
    expect(agentThreadStart).toHaveBeenCalledTimes(1);
    expect(live!.body).toMatchObject({ thread: "01JNEW" });
    expect(live!.body.messages).toHaveLength(1);
  });

  it("opens the thread a trigger run links to", async () => {
    render(() => <AgentChat agent="triage" label="Triage" thread="01JTHREAD" />);
    await waitFor(() => expect(agentThread).toHaveBeenCalledWith("triage", "01JTHREAD"));
    const convo = within(await screen.findByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(convo.getByText("Tidy my list.")).toBeTruthy());
  });

  it("reopens a thread with its turns and its pending card, and records Apply in the thread", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = within(screen.getByRole("complementary", { name: "Threads" }));
    fireEvent.click(await list.findByRole("button", { name: /^Groceries/ }));
    await waitFor(() => expect(agentThread).toHaveBeenCalledWith("triage", "01JTHREAD"));
    const convo = within(await screen.findByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(convo.getByText("Tidy my list.")).toBeTruthy());
    expect(convo.getByText("Here is an edit.")).toBeTruthy();
    const card = within(convo.getByRole("region", { name: /Proposed edit to Groceries/ }));
    fireEvent.click(card.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(aiApply).toHaveBeenCalled());
    expect(aiApply.mock.calls[0]![0]).toMatchObject({ id: "toolu_p", thread: "01JTHREAD", agent: "triage" });
    await waitFor(() => expect(card.getByRole("status")).toHaveTextContent("Applied."));
    // The next message leaves the outcome to the server, which keeps it in the thread.
    await send("Thanks.");
    expect(live!.body).toMatchObject({ thread: "01JTHREAD" });
    expect(live!.body.messages).toEqual([{ role: "user", content: [{ type: "text", text: "Thanks." }] }]);
  });

  it("records Discard in the thread", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = within(screen.getByRole("complementary", { name: "Threads" }));
    fireEvent.click(await list.findByRole("button", { name: /^Groceries/ }));
    const convo = within(await screen.findByRole("list", { name: "Conversation" }));
    const card = within(await convo.findByRole("region", { name: /Proposed edit to Groceries/ }));
    fireEvent.click(card.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(aiDiscard).toHaveBeenCalledWith("toolu_p", "01JTHREAD"));
    expect(card.getByRole("status")).toHaveTextContent("Discarded.");
  });

  it("shows a stored outcome as it stands and no buttons for it", async () => {
    agentThread.mockResolvedValue(thread({ pending: 0, proposals: [{ event: "proposal", data: PROPOSAL, status: "discarded" }] }));
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = within(screen.getByRole("complementary", { name: "Threads" }));
    fireEvent.click(await list.findByRole("button", { name: /^Groceries/ }));
    const convo = within(await screen.findByRole("list", { name: "Conversation" }));
    const card = within(await convo.findByRole("region", { name: /Proposed edit to Groceries/ }));
    expect(card.getByRole("status")).toHaveTextContent("Discarded.");
    expect(card.queryByRole("button", { name: "Apply" })).toBeNull();
  });
});

describe("thread actions", () => {
  async function rows() {
    const list = within(screen.getByRole("complementary", { name: "Threads" }));
    await waitFor(() => expect(list.getAllByRole("listitem")).toHaveLength(2));
    return list;
  }

  function openMenu(list: ReturnType<typeof within>, title: string) {
    fireEvent.click(list.getByRole("button", { name: `Actions for ${title}` }));
    return within(screen.getByRole("menu", { name: `Actions for ${title}` }));
  }

  it("renames a thread inline", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = await rows();
    fireEvent.click(openMenu(list, "Groceries").getByRole("menuitem", { name: "Rename" }));
    const input = list.getByLabelText("Thread title") as HTMLInputElement;
    expect(input.value).toBe("Groceries");
    const calls = agentThreads.mock.calls.length;
    fireEvent.input(input, { target: { value: "Shopping" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(renameAgentThread).toHaveBeenCalledWith("triage", "01JTHREAD", "Shopping"));
    await waitFor(() => expect(agentThreads.mock.calls.length).toBeGreaterThan(calls));
    expect(list.queryByLabelText("Thread title")).toBeNull();
  });

  it("leaves the title as it was when renaming is given up", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = await rows();
    fireEvent.click(openMenu(list, "Roof").getByRole("menuitem", { name: "Rename" }));
    fireEvent.keyDown(list.getByLabelText("Thread title"), { key: "Escape" });
    expect(list.queryByLabelText("Thread title")).toBeNull();
    expect(renameAgentThread).not.toHaveBeenCalled();
  });

  it("archives a thread through the note route at its current version", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = await rows();
    agentThreads.mockResolvedValue({ threads: [{ id: "01JOLDER", title: "Roof", pending: 0 }] });
    fireEvent.click(openMenu(list, "Groceries").getByRole("menuitem", { name: "Archive" }));
    await waitFor(() => expect(archiveNote).toHaveBeenCalledWith("01JTHREAD", "t1"));
    await waitFor(() => expect(list.getAllByRole("listitem")).toHaveLength(1));
    expect(list.queryByText("Groceries")).toBeNull();
  });

  it("links a thread's menu to the agent's AI activity in the Log (activity Y5)", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = await rows();
    const menu = openMenu(list, "Groceries");
    expect(menu.getAllByRole("menuitem").map((m) => m.textContent)).toEqual(["Rename", "Archive", "Show in log", "Delete"]);
    const link = menu.getByRole("menuitem", { name: "Show in log" });
    expect(link.getAttribute("href")).toBe("/log?category=ai&actor=agent:triage");
    fireEvent.click(link);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("opens a thread's menu on right-click", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = await rows();
    fireEvent.contextMenu(list.getByRole("button", { name: /^Roof/ }));
    expect(screen.getByRole("menu", { name: "Actions for Roof" })).toBeTruthy();
  });

  it("shows archived threads dimmed with Show archived, and unarchives one", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = await rows();
    agentThreads.mockResolvedValue({
      threads: [
        { id: "01JTHREAD", title: "Groceries", pending: 0 },
        { id: "01JOLDER", title: "Roof", pending: 0 },
        { id: "01JFILED", title: "Gutters", pending: 0, archived: true },
      ],
    });
    fireEvent.click(list.getByLabelText("Show archived"));
    await waitFor(() => expect(agentThreads).toHaveBeenLastCalledWith("triage", true));
    await waitFor(() => expect(list.getAllByRole("listitem")).toHaveLength(3));
    const filed = list.getByRole("button", { name: /^Gutters/ });
    expect(filed).toHaveTextContent("(archived)");
    expect(filed.closest(".agent-thread-row")).toHaveClass("agent-thread-archived");
    expect(list.getByRole("button", { name: /^Roof/ }).closest(".agent-thread-row")).not.toHaveClass("agent-thread-archived");
    agentThread.mockResolvedValue(thread({ id: "01JFILED", version: "g3", archived: true }));
    fireEvent.click(openMenu(list, "Gutters").getByRole("menuitem", { name: "Unarchive" }));
    await waitFor(() => expect(unarchiveNote).toHaveBeenCalledWith("01JFILED", "g3"));
    expect(archiveNote).not.toHaveBeenCalled();
    // Off again, the list leaves archived threads out.
    fireEvent.click(list.getByLabelText("Show archived"));
    await waitFor(() => expect(agentThreads).toHaveBeenLastCalledWith("triage"));
  });

  it("deletes a thread after confirming, and the open one gives way to a new thread", async () => {
    const ask = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = await rows();
    fireEvent.click(list.getByRole("button", { name: /^Groceries/ }));
    const convo = within(await screen.findByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(convo.getByText("Tidy my list.")).toBeTruthy());
    agentThreads.mockResolvedValue({ threads: [{ id: "01JOLDER", title: "Roof", pending: 0 }] });
    fireEvent.click(openMenu(list, "Groceries").getByRole("menuitem", { name: "Delete" }));
    expect(ask).toHaveBeenCalledWith(expect.stringContaining("trash"));
    await waitFor(() => expect(deleteAgentThread).toHaveBeenCalledWith("triage", "01JTHREAD"));
    await waitFor(() => expect(list.getAllByRole("listitem")).toHaveLength(1));
    await waitFor(() => expect(screen.queryByText("Tidy my list.")).toBeNull());
    ask.mockRestore();
  });

  it("keeps a thread when Delete is not confirmed", async () => {
    const ask = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = await rows();
    fireEvent.click(openMenu(list, "Roof").getByRole("menuitem", { name: "Delete" }));
    expect(deleteAgentThread).not.toHaveBeenCalled();
    ask.mockRestore();
  });

  it("shows why an action failed", async () => {
    archiveNote.mockRejectedValue(new Error("The note changed."));
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = await rows();
    fireEvent.click(openMenu(list, "Roof").getByRole("menuitem", { name: "Archive" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("The note changed."));
  });
});

describe("a stored thread as turns", () => {
  it("splits the outcome lines from the question and fills each step from its summary", () => {
    const turns = turnsFromThread(
      thread({
        messages: [
          {
            role: "user",
            content: [
              { type: "text", text: "[Applied by the user: edit of note 01JNOTE now at version v2]" },
              { type: "text", text: "What next?" },
            ],
          },
          { role: "assistant", content: [{ type: "tool_call", id: "toolu_p", name: "search_notes", input: { text: "x" } }] },
          { role: "user", content: [{ type: "tool_result", id: "toolu_p", content: "{}", is_error: false }] },
          { role: "assistant", content: [{ type: "text", text: "Nothing." }, { type: "notice", kind: "cut_off", text: "Cut off." }] },
        ],
        proposals: [],
      }),
    );
    expect(turns).toHaveLength(2);
    expect(turns[0]).toMatchObject({ role: "user", text: "What next?", outcomes: ["[Applied by the user: edit of note 01JNOTE now at version v2]"] });
    const parts = turns[1]!.parts;
    expect(parts[0]).toMatchObject({ kind: "tool", name: "search_notes", done: true, notes: [{ id: "01JNOTE", title: "Groceries" }] });
    expect(parts[1]).toEqual({ kind: "text", text: "Nothing." });
    expect(parts[2]).toMatchObject({ kind: "notice", notice: "cut_off" });
  });
});

describe("the AI panel's agent picker", () => {
  async function withAgents() {
    agentsList.mockResolvedValue({
      agents: [
        { name: "assistant", label: "Assistant", built_in: true, enabled: true },
        { name: "triage", label: "Triage", built_in: false, enabled: true, provider: "anthropic", model: "claude-sonnet-5" },
        { name: "sleepy", label: "Sleepy", built_in: false, enabled: false },
      ],
      tools: [],
    });
    await refreshAgents();
  }

  it("offers Assistant first and every enabled agent, and keeps each conversation as a thread", async () => {
    await withAgents();
    agentThreadStart.mockResolvedValue({ id: "01JPANEL", title: "New thread", agent: "assistant", version: "p1", pending: 0 });
    render(() => <ChatPanel noteId={() => ""} />);
    const picker = screen.getByLabelText("Agent") as HTMLSelectElement;
    expect([...picker.options].map((o) => o.textContent)).toEqual(["Assistant", "Triage"]);
    expect(picker).toHaveValue("assistant");
    await send("Hello");
    expect(agentThreadStart).toHaveBeenCalledWith("assistant");
    expect(live!.body).toMatchObject({ agent: "assistant", thread: "01JPANEL", messages: [{ role: "user" }] });
    expect(live!.body.messages).toHaveLength(1);
  });

  async function threadThen(state: "archived" | "deleted" | "open") {
    await withAgents();
    agentThreadStart.mockResolvedValue({ id: "01JPANEL", title: "New thread", agent: "assistant", version: "p1", pending: 0 });
    render(() => <ChatPanel noteId={() => ""} />);
    await send("Hello");
    live!.onEvent({ event: "text", data: { delta: "Hi." } });
    live!.onEvent({ event: "done", data: { stop: "end_turn", messages: [] } });
    live!.end();
    await waitFor(() => expect(screen.getByRole("button", { name: "Send" })).toBeTruthy());
    live = null;
    const { ApiError } = await import("../api");
    if (state === "deleted") agentThread.mockRejectedValue(new ApiError(404, { error: "not_found", message: "there is no agent thread 01JPANEL" }));
    else agentThread.mockResolvedValue(thread({ id: "01JPANEL", agent: "assistant", archived: state === "archived" }));
    agentThreadStart.mockResolvedValue({ id: "01JFRESH", title: "New thread", agent: "assistant", version: "f1", pending: 0 });
    await send("Again");
    expect(agentThread).toHaveBeenCalledWith("assistant", "01JPANEL");
  }

  it("starts a new thread when its thread was archived elsewhere", async () => {
    await threadThen("archived");
    expect(live!.body).toMatchObject({ thread: "01JFRESH" });
    expect(agentThreadStart).toHaveBeenCalledTimes(2);
    expect(screen.queryByText("Hello")).toBeNull();
  });

  it("starts a new thread when its thread was deleted elsewhere", async () => {
    await threadThen("deleted");
    expect(live!.body).toMatchObject({ thread: "01JFRESH" });
    expect(agentThreadStart).toHaveBeenCalledTimes(2);
  });

  it("stays on its thread while it is open", async () => {
    await threadThen("open");
    expect(live!.body).toMatchObject({ thread: "01JPANEL" });
    expect(agentThreadStart).toHaveBeenCalledTimes(1);
  });

  it("runs the chosen agent, whose own model replaces the picker", async () => {
    await withAgents();
    render(() => <ChatPanel noteId={() => ""} />);
    fireEvent.change(screen.getByLabelText("Agent"), { target: { value: "triage" } });
    await waitFor(() => expect(screen.getByText("Model: claude-sonnet-5")).toBeTruthy());
    expect(screen.queryByLabelText("Provider")).toBeNull();
    await send("Sort my inbox");
    expect(agentThreadStart).toHaveBeenCalledWith("triage");
    expect(live!.body).toMatchObject({ agent: "triage", thread: "01JNEW" });
  });
});

describe("turns in the background (agents G17)", () => {
  afterEach(() => resetRunning());

  /** Thread 01JTHREAD with turn `turn` running on it since `startedMs`. */
  function running(turn: string, startedMs: number): AgentThread {
    const t = thread();
    return {
      ...t,
      running: true,
      turn,
      started_ms: startedMs,
      // The turn's question, and a round the server already wrote, which reading the turn again
      // brings back.
      messages: [
        ...t.messages!,
        { role: "user", content: [{ type: "text", text: "More please." }] },
        { role: "assistant", content: [{ type: "text", text: "Partial" }] },
      ],
    };
  }

  const ROOF = thread({
    id: "01JOLDER",
    title: "Roof",
    pending: 0,
    messages: [{ role: "user", content: [{ type: "text", text: "Slate?" }] }],
    proposals: [],
  });

  it("leaves a turn running when another thread is opened, and shows it still answering on return", async () => {
    agentThread.mockImplementation(async (_agent: string, id: string) => (id === "01JOLDER" ? ROOF : thread()));
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = within(screen.getByRole("complementary", { name: "Threads" }));
    fireEvent.click(await list.findByRole("button", { name: /^Groceries/ }));
    const convo = () => within(screen.getByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(convo().getByText("Tidy my list.")).toBeTruthy());
    await send("More please.");
    live!.onEvent({ event: "text", data: { delta: "Partial" } });
    // Another thread can be opened while it answers: the turn runs on.
    const roof = list.getByRole("button", { name: /^Roof/ });
    expect(roof).not.toBeDisabled();
    fireEvent.click(roof);
    await waitFor(() => expect(convo().getByText("Slate?")).toBeTruthy());
    expect(turns.aiTurnStop).not.toHaveBeenCalled();
    expect(screen.queryByText("Stopped.")).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
    // What it says meanwhile is kept: the thread's one subscription reads on (agents G18).
    live!.onEvent({ event: "text", data: { delta: " and more" } });

    fireEvent.click(list.getByRole("button", { name: /^Groceries/ }));
    await waitFor(() => expect(convo().getByText("Partial and more")).toBeTruthy());
    expect(turns.streamTurn).toHaveBeenCalledTimes(1);
    expect(convo().getAllByText("More please.")).toHaveLength(1);
    expect(document.querySelector(".ai-activity")).toHaveTextContent(/Writing…/);
    expect(screen.getByRole("button", { name: "Stop" })).toBeTruthy();
    live!.onEvent({ event: "done", data: { stop: "end_turn", messages: [] } });
    live!.end();
    await waitFor(() => expect(document.querySelector(".ai-activity")).toBeNull());
    await waitFor(() => expect(screen.getByRole("button", { name: "Send" })).toBeTruthy());
  });

  it("follows a thread's running turn from its first event, with the activity line timed from its start", async () => {
    agentThread.mockImplementation(async (_agent: string, id: string) => (id === "01JTHREAD" ? running("turn-elsewhere", Date.now() - 5_000) : ROOF));
    turns.streamTurn.mockImplementationOnce(
      (_turn: string, _after: number, onEvent: (e: StreamEvent) => void, signal?: AbortSignal) =>
        new Promise<void>((resolve, reject) => {
          live = { onEvent, body: {}, end: resolve };
          signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
        }),
    );
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = within(screen.getByRole("complementary", { name: "Threads" }));
    fireEvent.click(await list.findByRole("button", { name: /^Groceries/ }));
    await waitFor(() => expect(live).not.toBeNull());
    const convo = () => within(screen.getByRole("list", { name: "Conversation" }));
    // The thread before the turn, then the turn read from its start.
    expect(turns.streamTurn).toHaveBeenLastCalledWith("turn-elsewhere", 0, expect.any(Function), expect.any(AbortSignal));
    await waitFor(() => expect(convo().getByText("Tidy my list.")).toBeTruthy());
    expect(convo().getAllByText("More please.")).toHaveLength(1);
    expect(convo().queryByText("Partial")).toBeNull();
    const line = () => document.querySelector(".ai-activity");
    expect(line()).toHaveTextContent(/Thinking… [5-9]s/);
    live!.onEvent({ event: "text", data: { delta: "Partial" } });
    await waitFor(() => expect(convo().getByText("Partial")).toBeTruthy());
    expect(line()).toHaveTextContent(/Writing…/);
    expect(screen.getByRole("button", { name: "Stop" })).toBeTruthy();
    live!.onEvent({ event: "done", data: { stop: "end_turn", messages: [] } });
    live!.end();
    await waitFor(() => expect(line()).toBeNull());
  });

  it("stops a running thread's turn on the server with Stop", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = within(screen.getByRole("complementary", { name: "Threads" }));
    fireEvent.click(await list.findByRole("button", { name: /^Groceries/ }));
    await waitFor(() => expect(screen.getByText("Tidy my list.")).toBeTruthy());
    await send("More please.");
    const { turn } = await turns.aiTurnStart.mock.results[0]!.value;
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(screen.getByText("Stopped.")).toBeTruthy());
    expect(turns.aiTurnStop).toHaveBeenCalledWith(turn);
  });

  it("marks a running thread with a spinner, and one that answered while closed with New reply until opened", async () => {
    agentThreads.mockResolvedValue({
      threads: [
        { id: "01JTHREAD", title: "Groceries", pending: 1, running: true, turn: "turn-x", started_ms: Date.now() },
        { id: "01JOLDER", title: "Roof", pending: 0 },
      ],
    });
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = within(screen.getByRole("complementary", { name: "Threads" }));
    const groceries = () => list.getByRole("button", { name: /^Groceries/ });
    await waitFor(() => expect(within(groceries()).getByRole("img", { name: "Running" })).toBeTruthy());
    expect(within(list.getByRole("button", { name: /^Roof/ })).queryByRole("img", { name: "Running" })).toBeNull();

    // The live stream says the turn ended: the list reads again, and the reply is new.
    agentThreads.mockResolvedValue({
      threads: [
        { id: "01JTHREAD", title: "Groceries", pending: 1 },
        { id: "01JOLDER", title: "Roof", pending: 0 },
      ],
    });
    liveTurn({ agent: "triage", thread: "01JTHREAD", turn: "turn-x", running: false });
    await waitFor(() => expect(within(groceries()).getByRole("img", { name: "New reply" })).toBeTruthy());
    expect(within(groceries()).queryByRole("img", { name: "Running" })).toBeNull();

    fireEvent.click(groceries());
    await waitFor(() => expect(within(groceries()).queryByRole("img", { name: "New reply" })).toBeNull());
    expect(hasNewReply("01JTHREAD")).toBe(false);
  });

  it("gives no New reply to a thread whose turn ends while it is open", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = within(screen.getByRole("complementary", { name: "Threads" }));
    fireEvent.click(await list.findByRole("button", { name: /^Groceries/ }));
    await waitFor(() => expect(screen.getByText("Tidy my list.")).toBeTruthy());
    liveTurn({ agent: "triage", thread: "01JTHREAD", turn: null, running: false });
    expect(hasNewReply("01JTHREAD")).toBe(false);
  });
});

describe("the AI panel as a view of agent threads (agents G18)", () => {
  afterEach(() => {
    resetRunning();
    localStorage.removeItem(PANEL_KEY);
  });

  async function withAgents() {
    agentsList.mockResolvedValue({
      agents: [
        { name: "assistant", label: "Assistant", built_in: true, enabled: true },
        { name: "triage", label: "Triage", built_in: false, enabled: true, provider: "anthropic", model: "claude-sonnet-5" },
      ],
      tools: [],
    });
    await refreshAgents();
  }

  const ROOF = thread({
    id: "01JOLDER",
    title: "Roof",
    pending: 0,
    messages: [{ role: "user", content: [{ type: "text", text: "Slate?" }] }],
    proposals: [],
  });

  type View = ReturnType<typeof within>;

  /** The panel, in an aside as the shell has it, and the Chat tab of Triage beside it. */
  function both(): { panel: View; tab: View } {
    agentThread.mockImplementation(async (_agent: string, id: string) => (id === "01JOLDER" ? ROOF : thread()));
    render(() => (
      <>
        <aside aria-label="AI">
          <ChatPanel noteId={() => "01JNOTE"} />
        </aside>
        <AgentChat agent="triage" label="Triage" />
      </>
    ));
    return {
      panel: within(screen.getByRole("complementary", { name: "AI" })),
      tab: within(screen.getByRole("region", { name: "Chat" })),
    };
  }

  /** In the panel, choose thread `title` from its thread picker. */
  async function pick(panel: View, title: RegExp | string) {
    fireEvent.click(await panel.findByRole("button", { name: /^Thread: / }));
    const menu = within(await panel.findByRole("menu", { name: "Threads" }));
    fireEvent.click(typeof title === "string" ? menu.getByRole("menuitem", { name: title }) : await menu.findByRole("menuitemradio", { name: title }));
  }

  async function sendIn(view: View, text: string) {
    fireEvent.input(view.getByLabelText("Ask AI"), { target: { value: text } });
    fireEvent.click(view.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(live).not.toBeNull());
  }

  it("lists the agent's threads with running and new-reply marks, and shows the one chosen", async () => {
    await withAgents();
    agentThreads.mockResolvedValue({
      threads: [
        { id: "01JTHREAD", title: "Groceries", pending: 1, running: true, turn: "turn-x", started_ms: Date.now() },
        { id: "01JOLDER", title: "Roof", pending: 0 },
      ],
    });
    const { panel } = both();
    fireEvent.change(panel.getByLabelText("Agent"), { target: { value: "triage" } });
    liveTurn({ agent: "triage", thread: "01JOLDER", turn: "turn-y", running: false });
    fireEvent.click(await panel.findByRole("button", { name: "Thread: New thread" }));
    const menu = within(await panel.findByRole("menu", { name: "Threads" }));
    await waitFor(() => expect(menu.getAllByRole("menuitemradio")).toHaveLength(2));
    const [groceries, roof] = menu.getAllByRole("menuitemradio");
    expect(menu.getByRole("menuitem")).toHaveTextContent("New thread");
    expect(within(groceries!).getByRole("img", { name: "Running" })).toBeTruthy();
    expect(within(roof!).getByRole("img", { name: "New reply" })).toBeTruthy();
    expect(within(roof!).queryByRole("img", { name: "Running" })).toBeNull();

    fireEvent.click(roof!);
    const convo = within(panel.getByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(convo.getByText("Slate?")).toBeTruthy());
    expect(agentThread).toHaveBeenCalledWith("triage", "01JOLDER");
    expect(hasNewReply("01JOLDER")).toBe(false);
    await waitFor(() => expect(panel.getByRole("button", { name: "Thread: Roof" })).toBeTruthy());
  });

  it("starts a new thread from the picker, and opens the shown one in the agent screen", async () => {
    await withAgents();
    const { panel } = both();
    fireEvent.change(panel.getByLabelText("Agent"), { target: { value: "triage" } });
    expect(await panel.findByRole("button", { name: "Open in agent screen" })).toBeDisabled();
    await pick(panel, /^Groceries/);
    const convo = () => within(panel.getByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(convo().getByText("Tidy my list.")).toBeTruthy());
    fireEvent.click(panel.getByRole("button", { name: "Open in agent screen" }));
    expect(navigatePane).toHaveBeenCalledWith(expect.any(String), "/agents/triage/chat?thread=01JTHREAD");

    await pick(panel, "New thread");
    await waitFor(() => expect(convo().queryByText("Tidy my list.")).toBeNull());
    expect(panel.getByRole("button", { name: "Thread: New thread" })).toBeTruthy();
    await sendIn(panel, "Sort my inbox");
    expect(live!.body).toMatchObject({ agent: "triage", thread: "01JNEW", note_id: "01JNOTE" });
  });

  it("shows one stream in the panel and the Chat tab, and a Stop in one ends it in both", async () => {
    await withAgents();
    const { panel, tab } = both();
    fireEvent.change(panel.getByLabelText("Agent"), { target: { value: "triage" } });
    await pick(panel, /^Groceries/);
    fireEvent.click(await tab.findByRole("button", { name: /^Groceries/ }));
    const inPanel = () => within(panel.getByRole("list", { name: "Conversation" }));
    const inTab = () => within(tab.getByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(inPanel().getByText("Tidy my list.")).toBeTruthy());
    await waitFor(() => expect(inTab().getByText("Tidy my list.")).toBeTruthy());
    // One conversation: the thread was read once.
    expect(agentThread).toHaveBeenCalledTimes(1);

    // The panel adds the open note; the reply streams in both.
    await sendIn(panel, "More please.");
    expect(live!.body).toMatchObject({ agent: "triage", thread: "01JTHREAD", note_id: "01JNOTE" });
    live!.onEvent({ event: "text", data: { delta: "Partial" } });
    await waitFor(() => expect(inPanel().getByText("Partial")).toBeTruthy());
    expect(inTab().getByText("Partial")).toBeTruthy();
    expect(inTab().getAllByText("More please.")).toHaveLength(1);
    expect(turns.streamTurn).toHaveBeenCalledTimes(1);

    const { turn } = await turns.aiTurnStart.mock.results[0]!.value;
    fireEvent.click(tab.getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(inPanel().getByText("Stopped.")).toBeTruthy());
    expect(inTab().getByText("Stopped.")).toBeTruthy();
    expect(turns.aiTurnStop).toHaveBeenCalledWith(turn);
    expect(panel.getByRole("button", { name: "Send" })).toBeTruthy();
    expect(tab.getByRole("button", { name: "Send" })).toBeTruthy();
  });

  it("sends no note from the Chat tab", async () => {
    await withAgents();
    const { tab } = both();
    fireEvent.click(await tab.findByRole("button", { name: /^Groceries/ }));
    await waitFor(() => expect(within(tab.getByRole("list", { name: "Conversation" })).getByText("Tidy my list.")).toBeTruthy());
    await sendIn(tab, "More please.");
    expect(live!.body).toMatchObject({ thread: "01JTHREAD" });
    expect(live!.body).not.toHaveProperty("note_id");
  });

  it("switching the panel's thread leaves the Chat tab's alone", async () => {
    await withAgents();
    const { panel, tab } = both();
    fireEvent.change(panel.getByLabelText("Agent"), { target: { value: "triage" } });
    await pick(panel, /^Groceries/);
    fireEvent.click(await tab.findByRole("button", { name: /^Groceries/ }));
    const inPanel = () => within(panel.getByRole("list", { name: "Conversation" }));
    const inTab = () => within(tab.getByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(inTab().getByText("Tidy my list.")).toBeTruthy());

    await pick(panel, /^Roof/);
    await waitFor(() => expect(inPanel().getByText("Slate?")).toBeTruthy());
    expect(inTab().getByText("Tidy my list.")).toBeTruthy();
    expect(tab.getByRole("button", { name: /^Groceries/ })).toHaveAttribute("aria-current", "true");

    // And the other way round.
    fireEvent.click(tab.getByRole("button", { name: "New thread" }));
    await waitFor(() => expect(inTab().queryByText("Tidy my list.")).toBeNull());
    expect(inPanel().getByText("Slate?")).toBeTruthy();
  });

  it("in an app, polls a thread open in both views once, and shows its events in both", async () => {
    inApp.now = { server_url: "https://home.example" };
    try {
      await withAgents();
      let finish: (v: unknown) => void = () => undefined;
      aiTurnPoll
        .mockResolvedValueOnce({ events: [{ event: "text", data: { delta: "Polled" } }], next: 1, done: false })
        .mockImplementationOnce(() => new Promise((r) => (finish = r)));
      const { panel, tab } = both();
      fireEvent.change(panel.getByLabelText("Agent"), { target: { value: "triage" } });
      await pick(panel, /^Groceries/);
      fireEvent.click(await tab.findByRole("button", { name: /^Groceries/ }));
      const inPanel = () => within(panel.getByRole("list", { name: "Conversation" }));
      const inTab = () => within(tab.getByRole("list", { name: "Conversation" }));
      await waitFor(() => expect(inTab().getByText("Tidy my list.")).toBeTruthy());
      fireEvent.input(tab.getByLabelText("Ask AI"), { target: { value: "More please." } });
      fireEvent.click(tab.getByRole("button", { name: "Send" }));
      await waitFor(() => expect(inPanel().getByText("Polled")).toBeTruthy());
      expect(inTab().getByText("Polled")).toBeTruthy();
      await waitFor(() => expect(aiTurnPoll).toHaveBeenCalledTimes(2));
      const { turn } = await turns.aiTurnStart.mock.results[0]!.value;
      expect(aiTurnPoll.mock.calls.map((c) => c.slice(0, 2))).toEqual([
        [turn, 0],
        [turn, 1],
      ]);
      expect(turns.streamTurn).not.toHaveBeenCalled();
      finish({ events: [{ event: "done", data: { stop: "end_turn", messages: [] } }], next: 2, done: true });
      await waitFor(() => expect(panel.getByRole("button", { name: "Send" })).toBeTruthy());
      expect(tab.getByRole("button", { name: "Send" })).toBeTruthy();
      expect(aiTurnPoll).toHaveBeenCalledTimes(2);
    } finally {
      inApp.now = null;
    }
  });

  it("keeps the panel's agent and thread over a reload", async () => {
    await withAgents();
    const { panel } = both();
    fireEvent.change(panel.getByLabelText("Agent"), { target: { value: "triage" } });
    await pick(panel, /^Groceries/);
    await waitFor(() => expect(JSON.parse(localStorage.getItem(PANEL_KEY) ?? "null")).toEqual({ agent: "triage", thread: "01JTHREAD" }));

    // A reload: the panel starts afresh and takes its saved choice.
    cleanup();
    forgetPanelChoice();
    newConversation();
    setAgent("assistant");
    resetThreads();
    expect(JSON.parse(localStorage.getItem(PANEL_KEY) ?? "null")).toEqual({ agent: "triage", thread: "01JTHREAD" });
    agentThread.mockClear();
    const again = both().panel;
    await waitFor(() => expect(within(again.getByRole("list", { name: "Conversation" })).getByText("Tidy my list.")).toBeTruthy());
    expect(again.getByLabelText("Agent")).toHaveValue("triage");
    expect(agentThread).toHaveBeenCalledWith("triage", "01JTHREAD");
  });

  it("opens to a new thread when the saved one was archived since", async () => {
    await withAgents();
    localStorage.setItem(PANEL_KEY, JSON.stringify({ agent: "triage", thread: "01JTHREAD" }));
    forgetPanelChoice();
    agentThread.mockResolvedValue(thread({ archived: true }));
    render(() => <ChatPanel noteId={() => ""} />);
    await waitFor(() => expect(agentThread).toHaveBeenCalledWith("triage", "01JTHREAD"));
    await waitFor(() => expect(screen.getByRole("button", { name: "Thread: New thread" })).toBeTruthy());
    expect(screen.getByLabelText("Agent")).toHaveValue("triage");
    expect(screen.queryByText("Tidy my list.")).toBeNull();
    await waitFor(() => expect(JSON.parse(localStorage.getItem(PANEL_KEY) ?? "null")).toEqual({ agent: "triage", thread: null }));
  });
});
