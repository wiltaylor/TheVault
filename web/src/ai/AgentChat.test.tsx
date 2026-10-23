// The agent screen's Chat tab (agents G5, G7, G8) and the AI panel's agent picker (agents G6):
// the thread list with its pending badges, New thread, a thread started by the first message
// with only that message sent, reopened threads with their pending cards, outcomes recorded in
// the thread, and the panel's conversations as threads of the chosen agent.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentThread, StreamEvent } from "../api";

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

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    streamEvents,
    api: { ...real.api, agentThreads, agentThread, agentThreadStart, aiApply, aiDiscard, aiModels, aiProviders, agents: agentsList, app },
  };
});
vi.mock("../inApp", () => ({ appInfo: async () => null }));

const { AgentChat } = await import("./AgentChat");
const { ChatPanel } = await import("./ChatPanel");
const { newConversation, setAgent, turnsFromThread } = await import("./store");
const { refreshAgents } = await import("../agents");

const PROVIDERS = {
  default_provider: "anthropic",
  providers: {
    anthropic: { configured: true, model: "claude-opus-5" },
    openai: { configured: false, model: null },
    chatgpt: { configured: false, model: null },
  },
  confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true },
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
});

afterEach(async () => {
  newConversation();
  setAgent("assistant");
  live = null;
  cleanup();
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
    await waitFor(() => expect(list.getAllByRole("button", { name: /Groceries|Roof/ })).toHaveLength(2));
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
    fireEvent.click(await list.findByRole("button", { name: /Groceries/ }));
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
    fireEvent.click(await list.findByRole("button", { name: /Groceries/ }));
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
    fireEvent.click(await list.findByRole("button", { name: /Groceries/ }));
    const convo = within(await screen.findByRole("list", { name: "Conversation" }));
    const card = within(await convo.findByRole("region", { name: /Proposed edit to Groceries/ }));
    expect(card.getByRole("status")).toHaveTextContent("Discarded.");
    expect(card.queryByRole("button", { name: "Apply" })).toBeNull();
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
