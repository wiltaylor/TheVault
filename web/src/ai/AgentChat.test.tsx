// The agent screen's chat (agents G5, G7, G8) and the AI panel's agent picker (agents G6): the
// session picker with its pending badges, its search and New chat, Rename, Archive and Delete in
// the bar, a thread started by the first message with only that message sent, reopened threads
// with their pending cards, outcomes recorded in the thread, Enter sending and Shift+Enter
// starting a line, and the panel's conversations as threads of the chosen agent. The panel's
// thread picker, and a thread open in the panel and the agent screen at once as one conversation
// (agents G18).

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentThread, Json, StreamEvent } from "../api";
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
  confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true, agents: true, skills: true },
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

/** The user message the server keeps for a `/update-skills` turn (skills S6). */
const SKILLS_MSG: { role: "user"; content: Json[] } = {
  role: "user",
  content: [
    { type: "notice", kind: "skills_updated", text: "Skills updated (2)" },
    { type: "text", text: "The available skills changed; the list at the end of your instructions is current." },
  ],
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

type View = ReturnType<typeof within>;

/** Open the chat's session picker, in `view` when given, and give its list of chats. */
async function sessions(view: View = within(document.body)): Promise<View> {
  fireEvent.click(await view.findByRole("button", { name: /^Chat session: / }));
  return within(await view.findByRole("listbox", { name: "Chats" }));
}

/** Choose the chat `title` from the session picker. */
async function openSession(title: RegExp, view?: View) {
  const list = await sessions(view);
  fireEvent.click(await list.findByRole("option", { name: title }));
}

describe("the chat", () => {
  it("lists the agent's threads, newest first, with a badge on the one with pending proposals", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    await waitFor(() => expect(agentThreads).toHaveBeenCalledWith("triage"));
    const list = await sessions();
    await waitFor(() => expect(list.getAllByRole("option")).toHaveLength(3));
    const items = list.getAllByRole("option");
    expect(items[0]).toHaveTextContent("New chat");
    expect(items[0]).toHaveAttribute("aria-selected", "true");
    expect(items[1]).toHaveTextContent("Groceries");
    expect(items[2]).toHaveTextContent("Roof");
    expect(within(items[1]!).getByLabelText("1 proposal waits for you")).toHaveTextContent("1");
    expect(within(items[2]!).queryByLabelText(/wait for you|waits for you/)).toBeNull();
  });

  it("searches the chats by title, and Enter opens the one highlighted", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    await waitFor(() => expect(agentThreads).toHaveBeenCalled());
    const list = await sessions();
    const search = screen.getByRole("combobox", { name: "Search chats" });
    expect(search).toHaveFocus();
    await waitFor(() => expect(list.getAllByRole("option")).toHaveLength(3));
    fireEvent.input(search, { target: { value: "roo" } });
    await waitFor(() => expect(list.getAllByRole("option")).toHaveLength(2));
    expect(list.getAllByRole("option")[1]).toHaveTextContent("Roof");
    // The first match is highlighted, and Enter opens it.
    expect(search.getAttribute("aria-activedescendant")).toBe(list.getAllByRole("option")[1]!.id);
    fireEvent.keyDown(search, { key: "Enter" });
    await waitFor(() => expect(agentThread).toHaveBeenCalledWith("triage", "01JOLDER"));
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.getByRole("button", { name: /^Chat session: Roof/ })).toHaveFocus();
    // No match says so; Escape closes the list and gives focus back.
    await sessions();
    fireEvent.input(screen.getByRole("combobox", { name: "Search chats" }), { target: { value: "zebra" } });
    expect(await screen.findByText("No chat matches the search.")).toBeTruthy();
    fireEvent.keyDown(screen.getByRole("combobox", { name: "Search chats" }), { key: "Escape" });
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.getByRole("button", { name: /^Chat session: / })).toHaveFocus();
  });

  it("moves through the chats with the arrow keys, New chat first", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    await waitFor(() => expect(agentThreads).toHaveBeenCalled());
    const trigger = screen.getByRole("button", { name: "Chat session: New chat" });
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    const list = within(await screen.findByRole("listbox", { name: "Chats" }));
    await waitFor(() => expect(list.getAllByRole("option")).toHaveLength(3));
    const search = screen.getByRole("combobox", { name: "Search chats" });
    const active = () => document.getElementById(search.getAttribute("aria-activedescendant")!);
    expect(active()).toHaveTextContent("New chat");
    fireEvent.keyDown(search, { key: "ArrowDown" });
    expect(active()).toHaveTextContent("Groceries");
    fireEvent.keyDown(search, { key: "ArrowUp" });
    fireEvent.keyDown(search, { key: "ArrowUp" });
    expect(active()).toHaveTextContent("Roof");
    fireEvent.keyDown(search, { key: "ArrowDown" });
    expect(active()).toHaveTextContent("New chat");
  });

  it("starts a thread with the first message and sends only that message", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    await waitFor(() => expect(agentThreads).toHaveBeenCalled());
    fireEvent.click((await sessions()).getByRole("option", { name: /New chat/ }));
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

  it("sends on Enter, and Shift+Enter and Enter in an IME composition do not", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    await waitFor(() => expect(agentThreads).toHaveBeenCalled());
    const box = screen.getByLabelText("Ask AI");
    fireEvent.input(box, { target: { value: "Line one" } });
    expect(fireEvent.keyDown(box, { key: "Enter", shiftKey: true })).toBe(true);
    expect(fireEvent.keyDown(box, { key: "Enter", isComposing: true })).toBe(true);
    expect(fireEvent.keyDown(box, { key: "Enter", keyCode: 229 })).toBe(true);
    expect(turns.aiTurnStart).not.toHaveBeenCalled();
    fireEvent.input(box, { target: { value: "Line one\nLine two" } });
    expect(fireEvent.keyDown(box, { key: "Enter" })).toBe(false);
    await waitFor(() => expect(live).not.toBeNull());
    expect(live!.body.messages).toEqual([{ role: "user", content: [{ type: "text", text: "Line one\nLine two" }] }]);
  });

  it("shows the provider and model picker at the foot of the message box, and a turn uses the model chosen", async () => {
    aiModels.mockResolvedValue({
      provider: "anthropic",
      model: "claude-opus-5",
      models: [
        { id: "claude-opus-5", display_name: "Claude Opus 5" },
        { id: "claude-sonnet-5", display_name: "Claude Sonnet 5" },
      ],
    });
    render(() => <AgentChat agent="triage" label="Triage" />);
    const model = (await screen.findByLabelText("Model")) as HTMLSelectElement;
    expect(model.closest(".ai-form-foot")).toContainElement(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(model.value).toBe("claude-opus-5"));
    fireEvent.change(model, { target: { value: "claude-sonnet-5" } });
    await send("Which model?");
    expect(live!.body).toMatchObject({ provider: "anthropic", model: "claude-sonnet-5" });
  });

  it("opens the thread a trigger run links to, and says which thread it shows", async () => {
    const onThread = vi.fn();
    render(() => <AgentChat agent="triage" label="Triage" thread="01JTHREAD" onThread={onThread} />);
    await waitFor(() => expect(agentThread).toHaveBeenCalledWith("triage", "01JTHREAD"));
    const convo = within(await screen.findByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(convo.getByText("Tidy my list.")).toBeTruthy());
    await waitFor(() => expect(screen.getByRole("button", { name: /^Chat session: Groceries/ })).toBeTruthy());
    await openSession(/^Roof/);
    expect(onThread).toHaveBeenLastCalledWith("01JOLDER");
    fireEvent.click((await sessions()).getByRole("option", { name: /New chat/ }));
    expect(onThread).toHaveBeenLastCalledWith(null);
  });

  it("reopens a thread with its turns and its pending card, and records Apply in the thread", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    await openSession(/^Groceries/);
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
    await openSession(/^Groceries/);
    const convo = within(await screen.findByRole("list", { name: "Conversation" }));
    const card = within(await convo.findByRole("region", { name: /Proposed edit to Groceries/ }));
    fireEvent.click(card.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(aiDiscard).toHaveBeenCalledWith("toolu_p", "01JTHREAD"));
    expect(card.getByRole("status")).toHaveTextContent("Discarded.");
  });

  it("shows a stored outcome as it stands and no buttons for it", async () => {
    agentThread.mockResolvedValue(thread({ pending: 0, proposals: [{ event: "proposal", data: PROPOSAL, status: "discarded" }] }));
    render(() => <AgentChat agent="triage" label="Triage" />);
    await openSession(/^Groceries/);
    const convo = within(await screen.findByRole("list", { name: "Conversation" }));
    const card = within(await convo.findByRole("region", { name: /Proposed edit to Groceries/ }));
    expect(card.getByRole("status")).toHaveTextContent("Discarded.");
    expect(card.queryByRole("button", { name: "Apply" })).toBeNull();
  });
});

describe("the chosen chat's actions", () => {
  /** The chat with thread `title` open. */
  async function opened(title: RegExp = /^Groceries/) {
    render(() => <AgentChat agent="triage" label="Triage" />);
    await openSession(title);
    await waitFor(() => expect(screen.getByRole("button", { name: "Archive chat" })).toBeEnabled());
  }

  it("has nothing to rename, archive or delete in a new chat", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    await waitFor(() => expect(agentThreads).toHaveBeenCalled());
    expect(screen.getByRole("button", { name: "Rename chat" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Archive chat" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Delete chat" })).toBeDisabled();
  });

  it("renames the chat inline", async () => {
    await opened();
    fireEvent.click(screen.getByRole("button", { name: "Rename chat" }));
    const input = screen.getByLabelText("Thread title") as HTMLInputElement;
    expect(input.value).toBe("Groceries");
    const calls = agentThreads.mock.calls.length;
    fireEvent.input(input, { target: { value: "Shopping" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(renameAgentThread).toHaveBeenCalledWith("triage", "01JTHREAD", "Shopping"));
    await waitFor(() => expect(agentThreads.mock.calls.length).toBeGreaterThan(calls));
    expect(screen.queryByLabelText("Thread title")).toBeNull();
  });

  it("leaves the title as it was when renaming is given up", async () => {
    await opened(/^Roof/);
    fireEvent.click(screen.getByRole("button", { name: "Rename chat" }));
    fireEvent.keyDown(screen.getByLabelText("Thread title"), { key: "Escape" });
    expect(screen.queryByLabelText("Thread title")).toBeNull();
    expect(renameAgentThread).not.toHaveBeenCalled();
  });

  it("archives the chat through the note route at its current version, and unarchives it", async () => {
    await opened();
    agentThreads.mockResolvedValue({ threads: [{ id: "01JOLDER", title: "Roof", pending: 0 }] });
    fireEvent.click(screen.getByRole("button", { name: "Archive chat" }));
    await waitFor(() => expect(archiveNote).toHaveBeenCalledWith("01JTHREAD", "t1"));
    // The chat stays open, marked archived, and the list leaves it out.
    const unarchive = await screen.findByRole("button", { name: "Unarchive chat" });
    expect(screen.getByRole("button", { name: /^Chat session: Groceries/ })).toHaveTextContent("Archived");
    const list = await sessions();
    await waitFor(() => expect(list.getAllByRole("option")).toHaveLength(2));
    expect(list.queryByText("Groceries")).toBeNull();
    fireEvent.keyDown(screen.getByRole("combobox", { name: "Search chats" }), { key: "Escape" });
    agentThread.mockResolvedValue(thread({ version: "t2", archived: true }));
    fireEvent.click(unarchive);
    await waitFor(() => expect(unarchiveNote).toHaveBeenCalledWith("01JTHREAD", "t2"));
    expect(await screen.findByRole("button", { name: "Archive chat" })).toBeTruthy();
  });

  it("shows archived chats dimmed with Show archived", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    const list = await sessions();
    await waitFor(() => expect(list.getAllByRole("option")).toHaveLength(3));
    agentThreads.mockResolvedValue({
      threads: [
        { id: "01JTHREAD", title: "Groceries", pending: 0 },
        { id: "01JOLDER", title: "Roof", pending: 0 },
        { id: "01JFILED", title: "Gutters", pending: 0, archived: true },
      ],
    });
    fireEvent.click(screen.getByLabelText("Show archived"));
    await waitFor(() => expect(agentThreads).toHaveBeenLastCalledWith("triage", true));
    await waitFor(() => expect(list.getAllByRole("option")).toHaveLength(4));
    const filed = list.getByRole("option", { name: /^Gutters/ });
    expect(filed).toHaveTextContent("(archived)");
    expect(filed).toHaveClass("agent-thread-archived");
    expect(list.getByRole("option", { name: /^Roof/ })).not.toHaveClass("agent-thread-archived");
    // Off again, the list leaves archived threads out.
    fireEvent.click(screen.getByLabelText("Show archived"));
    await waitFor(() => expect(agentThreads).toHaveBeenLastCalledWith("triage"));
  });

  it("deletes the chat after confirming, and gives way to a new chat", async () => {
    const ask = vi.spyOn(window, "confirm").mockReturnValue(true);
    await opened();
    const convo = within(await screen.findByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(convo.getByText("Tidy my list.")).toBeTruthy());
    agentThreads.mockResolvedValue({ threads: [{ id: "01JOLDER", title: "Roof", pending: 0 }] });
    fireEvent.click(screen.getByRole("button", { name: "Delete chat" }));
    expect(ask).toHaveBeenCalledWith(expect.stringContaining("trash"));
    await waitFor(() => expect(deleteAgentThread).toHaveBeenCalledWith("triage", "01JTHREAD"));
    await waitFor(() => expect(screen.queryByText("Tidy my list.")).toBeNull());
    expect(screen.getByRole("button", { name: "Chat session: New chat" })).toBeTruthy();
    const list = await sessions();
    await waitFor(() => expect(list.getAllByRole("option")).toHaveLength(2));
    ask.mockRestore();
  });

  it("keeps the chat when Delete is not confirmed", async () => {
    const ask = vi.spyOn(window, "confirm").mockReturnValue(false);
    await opened(/^Roof/);
    fireEvent.click(screen.getByRole("button", { name: "Delete chat" }));
    expect(deleteAgentThread).not.toHaveBeenCalled();
    ask.mockRestore();
  });

  it("shows why an action failed", async () => {
    archiveNote.mockRejectedValue(new Error("The note changed."));
    await opened(/^Roof/);
    fireEvent.click(screen.getByRole("button", { name: "Archive chat" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("The note changed."));
  });
});

describe("a stored thread as turns", () => {
  it("sends /update-skills as a turn with no message on the thread, and shows Skills updated", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    await openSession(/^Groceries/);
    const convo = within(await screen.findByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(convo.getByText("Tidy my list.")).toBeTruthy());
    await send("/update-skills");
    expect(live!.body).toMatchObject({ agent: "triage", thread: "01JTHREAD", update_skills: true });
    expect(live!.body.messages).toEqual([]);
    await waitFor(() => expect(convo.getByText("Updating skills…")).toBeTruthy());
    live!.onEvent({ event: "skills_updated", data: { count: 2, skills: ["brand", "pdf-forms"], message: SKILLS_MSG } });
    await waitFor(() => expect(convo.getByText("Skills updated (2)")).toBeTruthy());
    // No user bubble, and not the text the model reads.
    expect(convo.queryByText("/update-skills")).toBeNull();
    expect(convo.queryByText(/The available skills changed/)).toBeNull();
    live!.onEvent({ event: "text", data: { delta: "Noted." } });
    live!.onEvent({ event: "done", data: { stop: "end_turn", messages: [] } });
    live!.end();
    await waitFor(() => expect(screen.getByRole("button", { name: "Send" })).toBeInTheDocument());
    // Any other text starting with / is sent as typed.
    live = null;
    await send("/update-skills please");
    expect(live!.body.update_skills).toBeUndefined();
    expect(live!.body.messages).toEqual([{ role: "user", content: [{ type: "text", text: "/update-skills please" }] }]);
  });

  it("shows a stored /update-skills message as its notice", () => {
    const turns = turnsFromThread(thread({ messages: [SKILLS_MSG, { role: "assistant", content: [{ type: "text", text: "Noted." }] }], proposals: [] }));
    expect(turns[0]).toMatchObject({ role: "user", text: "", notice: "Skills updated (2)" });
  });

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
    await openSession(/^Groceries/);
    const convo = () => within(screen.getByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(convo().getByText("Tidy my list.")).toBeTruthy());
    await send("More please.");
    live!.onEvent({ event: "text", data: { delta: "Partial" } });
    // Another thread can be opened while it answers: the turn runs on.
    await openSession(/^Roof/);
    await waitFor(() => expect(convo().getByText("Slate?")).toBeTruthy());
    expect(turns.aiTurnStop).not.toHaveBeenCalled();
    expect(screen.queryByText("Stopped.")).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
    // What it says meanwhile is kept: the thread's one subscription reads on (agents G18).
    live!.onEvent({ event: "text", data: { delta: " and more" } });

    await openSession(/^Groceries/);
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
    await openSession(/^Groceries/);
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
    await openSession(/^Groceries/);
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
    const list = await sessions();
    const groceries = () => list.getByRole("option", { name: /^Groceries/ });
    await waitFor(() => expect(within(groceries()).getByRole("img", { name: "Running" })).toBeTruthy());
    expect(within(list.getByRole("option", { name: /^Roof/ })).queryByRole("img", { name: "Running" })).toBeNull();

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
    await waitFor(() => expect(hasNewReply("01JTHREAD")).toBe(false));
    const again = await sessions();
    expect(within(again.getByRole("option", { name: /^Groceries/ })).queryByRole("img", { name: "New reply" })).toBeNull();
  });

  it("gives no New reply to a thread whose turn ends while it is open", async () => {
    render(() => <AgentChat agent="triage" label="Triage" />);
    await openSession(/^Groceries/);
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

  /** The panel, in an aside as the shell has it, and the chat of Triage beside it. */
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
    expect(navigatePane).toHaveBeenCalledWith(expect.any(String), "/agents/triage?thread=01JTHREAD");

    await pick(panel, "New thread");
    await waitFor(() => expect(convo().queryByText("Tidy my list.")).toBeNull());
    expect(panel.getByRole("button", { name: "Thread: New thread" })).toBeTruthy();
    await sendIn(panel, "Sort my inbox");
    expect(live!.body).toMatchObject({ agent: "triage", thread: "01JNEW", note_id: "01JNOTE" });
  });

  it("shows one stream in the panel and the agent screen, and a Stop in one ends it in both", async () => {
    await withAgents();
    const { panel, tab } = both();
    fireEvent.change(panel.getByLabelText("Agent"), { target: { value: "triage" } });
    await pick(panel, /^Groceries/);
    await openSession(/^Groceries/, tab);
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

  it("sends no note from the agent screen", async () => {
    await withAgents();
    const { tab } = both();
    await openSession(/^Groceries/, tab);
    await waitFor(() => expect(within(tab.getByRole("list", { name: "Conversation" })).getByText("Tidy my list.")).toBeTruthy());
    await sendIn(tab, "More please.");
    expect(live!.body).toMatchObject({ thread: "01JTHREAD" });
    expect(live!.body).not.toHaveProperty("note_id");
  });

  it("switching the panel's thread leaves the agent screen's alone", async () => {
    await withAgents();
    const { panel, tab } = both();
    fireEvent.change(panel.getByLabelText("Agent"), { target: { value: "triage" } });
    await pick(panel, /^Groceries/);
    await openSession(/^Groceries/, tab);
    const inPanel = () => within(panel.getByRole("list", { name: "Conversation" }));
    const inTab = () => within(tab.getByRole("list", { name: "Conversation" }));
    await waitFor(() => expect(inTab().getByText("Tidy my list.")).toBeTruthy());

    await pick(panel, /^Roof/);
    await waitFor(() => expect(inPanel().getByText("Slate?")).toBeTruthy());
    expect(inTab().getByText("Tidy my list.")).toBeTruthy();
    expect(tab.getByRole("button", { name: /^Chat session: Groceries/ })).toBeTruthy();

    // And the other way round.
    fireEvent.click((await sessions(tab)).getByRole("option", { name: /New chat/ }));
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
      await openSession(/^Groceries/, tab);
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
