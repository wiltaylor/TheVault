// Sub agents in the chat (subagents U4, U6): each start_subagent call is a block with its task,
// its live status and time, Stop, its result once done and the proposals it raised; its title
// opens the session beside the chat; a reopened thread's blocks come from its list of sessions;
// and a session shows read only, with Stop while it runs.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentThread, StreamEvent } from "../api";
import { turnsOver } from "./fakeTurns";

type OnEvent = (e: StreamEvent) => void;

let live: { onEvent: OnEvent; end: () => void } | null = null;

const streamEvents = vi.fn(
  (_path: string, _body: unknown, onEvent: OnEvent, signal?: AbortSignal) =>
    new Promise<void>((resolve, reject) => {
      live = { onEvent, end: resolve };
      signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
    }),
);
const agentThread = vi.fn();
const aiModels = vi.fn();
const aiAccounts = vi.fn();
const app = vi.fn();
const turns = turnsOver(streamEvents);

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    streamEvents,
    streamTurn: turns.streamTurn,
    api: { ...real.api, aiTurnStart: turns.aiTurnStart, aiTurnStop: turns.aiTurnStop, agentThread, aiModels, aiAccounts, app },
  };
});
vi.mock("../inApp", () => ({ appInfo: async () => null }));
const openBeside = vi.fn();
vi.mock("../workspace/store", async (orig) => ({ ...(await orig<typeof import("../workspace/store")>()), openBeside }));

const { Conversation } = await import("./ChatPanel");
const { createChatSession, resetThreads, turnsFromThread } = await import("./store");

const PROPOSAL = {
  id: "toolu_p",
  op: "update",
  note_id: "01JNOTE",
  title: "Groceries",
  base_version: "v1",
  before: { fields: { title: "Groceries" }, blocks: [] },
  after: { fields: { title: "Groceries and more" }, blocks: [] },
  ops: [{ op: "field", name: "title", before: "Groceries", after: "Groceries and more" }],
};

const START = { task: "Research venues", instructions: null, agent: null, permissions: null, model: null };

function emit(event: string, data: unknown) {
  live!.onEvent({ event, data });
}

beforeEach(() => {
  app.mockResolvedValue(null);
  aiAccounts.mockResolvedValue({ default: "a1", accounts: [{ id: "a1", name: "Anthropic", kind: "anthropic", kind_label: "Anthropic API", model: "claude-opus-5", configured: true }] });
  aiModels.mockResolvedValue({ account: "a1", model: "claude-opus-5", models: [] });
});

afterEach(() => {
  live = null;
  cleanup();
  resetThreads();
  vi.clearAllMocks();
});

async function ask(text: string) {
  fireEvent.input(screen.getByLabelText("Ask AI"), { target: { value: text } });
  fireEvent.click(screen.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
}

describe("a sub agent's block", () => {
  it("shows its task and live status, its proposals under it, Stop, and its result", async () => {
    const session = createChatSession();
    render(() => <Conversation session={session} />);
    await ask("Plan the party");
    emit("tool", { id: "toolu_s", name: "start_subagent", input: START });
    const started = Date.now();
    emit("subagent", {
      id: "toolu_s",
      call: "toolu_s",
      thread: "01JSUB",
      turn: "turn-sub",
      agent: "assistant",
      task: "Research venues",
      n: 1,
      status: "running",
      rounds: 1,
      tools: 1,
      started_ms: started,
      tool: "search_notes",
    });
    const block = within(await screen.findByRole("region", { name: "Sub agent: Research venues" }));
    expect(block.getByRole("status")).toHaveTextContent("Running: Searching notes, round 1");
    // Its proposal stands under it, with Apply and Discard as the parent's own.
    emit("proposal", { ...PROPOSAL, subagent: "toolu_s", sub_thread: "01JSUB" });
    expect(await block.findByRole("region", { name: "Proposed edit to Groceries" })).toBeTruthy();
    expect(block.getByRole("button", { name: "Apply" })).toBeTruthy();
    // Stop stops it alone.
    fireEvent.click(block.getByRole("button", { name: "Stop" }));
    expect(turns.aiTurnStop).toHaveBeenCalledWith("turn-sub");
    // The title opens its session beside the chat.
    fireEvent.click(block.getByRole("link", { name: "Sub agent: Research venues" }));
    expect(openBeside).toHaveBeenCalledWith(expect.any(String), "/subagent/assistant/01JSUB", expect.any(Function));
    emit("subagent", { id: "toolu_s", status: "done", rounds: 2, tools: 1, ended_ms: started + 3000, tool: null });
    emit("tool_result", {
      id: "toolu_s",
      is_error: false,
      summary: { notes: [], subagent: { thread: "01JSUB", agent: "assistant", outcome: "done", rounds: 2, tools: 1, ms: 3200, answer: "The **hall** is free." } },
    });
    await waitFor(() => expect(block.getAllByRole("status")[0]).toHaveTextContent("done"));
    expect(block.getByText(/3s/)).toBeTruthy();
    expect(block.getByText(/2 rounds/)).toBeTruthy();
    expect(block.queryByRole("button", { name: "Stop" })).toBeNull();
    fireEvent.click(block.getByText("Result"));
    expect(block.getByText("hall").tagName).toBe("STRONG");
    live!.end();
  });

  it("names the agent a task was delegated to, and says when it failed", async () => {
    const session = createChatSession();
    render(() => <Conversation session={session} />);
    await ask("Write it up");
    emit("tool", { id: "toolu_w", name: "start_subagent", input: { ...START, task: "Write the invite", agent: "writer" } });
    const block = within(await screen.findByRole("region", { name: "Delegated to writer: Write the invite" }));
    emit("tool_result", {
      id: "toolu_w",
      is_error: true,
      summary: { notes: [], message: "The sub agent failed: the account is out of credit.", subagent: { outcome: "failed", rounds: 0, tools: 0 } },
    });
    await waitFor(() => expect(block.getAllByRole("status")[0]).toHaveTextContent("failed"));
    expect(block.getByText("The sub agent failed: the account is out of credit.")).toBeTruthy();
    live!.end();
  });
});

describe("a reopened thread", () => {
  it("fills each block from the thread's sessions, its proposals under it", () => {
    const t: AgentThread = {
      id: "01JPARENT",
      title: "Party",
      agent: "lead",
      version: "v1",
      pending: 1,
      messages: [
        { role: "user", content: [{ type: "text", text: "Plan the party" }] },
        { role: "assistant", content: [{ type: "tool_call", id: "toolu_s", name: "start_subagent", input: START }] },
        { role: "user", content: [{ type: "tool_result", id: "toolu_s", content: "{}" }] },
        { role: "assistant", content: [{ type: "text", text: "Planned." }] },
      ],
      steps: { toolu_s: { summary: { notes: [], subagent: { outcome: "done", answer: "The hall.", rounds: 1, tools: 0 } }, is_error: false } },
      subagents: [
        { call: "toolu_s", thread: "01JSUB", agent: "lead", task: "Research venues", n: 1, status: "done", rounds: 1, tools: 0, started_ms: 1, ended_ms: 2 },
      ],
      proposals: [{ event: "proposal", data: { ...PROPOSAL, subagent: "toolu_s", thread: "01JPARENT" }, status: "pending" }],
    };
    const parts = turnsFromThread(t)[1]!.parts;
    const step = parts.find((p) => p.kind === "tool");
    expect(step?.kind === "tool" && step.sub).toMatchObject({ thread: "01JSUB", agent: "lead", status: "done", answer: "The hall." });
    expect(parts.some((p) => p.kind === "proposal" && p.state === "open")).toBe(true);
  });
});

describe("a sub agent's session", () => {
  const session = (over: Partial<AgentThread> = {}): AgentThread => ({
    id: "01JSUB",
    title: "Research venues",
    agent: "lead",
    version: "s1",
    pending: 1,
    origin: "subagent:01JPARENT",
    parent: { thread: "01JPARENT", turn: "t1", call: "toolu_s" },
    messages: [
      { role: "user", content: [{ type: "text", text: "Research venues" }] },
      { role: "assistant", content: [{ type: "tool_call", id: "toolu_p", name: "propose_edit", input: {} }] },
      { role: "user", content: [{ type: "tool_result", id: "toolu_p", content: "{}" }] },
      { role: "assistant", content: [{ type: "text", text: "The hall is free." }] },
    ],
    proposals: [{ event: "proposal", data: { ...PROPOSAL, thread: "01JSUB" }, status: "pending" }],
    ...over,
  });

  it("shows its messages and proposals read only, with no box to ask in", async () => {
    agentThread.mockResolvedValue(session());
    const s = createChatSession({ agent: "lead" });
    render(() => <Conversation session={s} readOnly />);
    void s.openThread("01JSUB");
    expect(await screen.findByText("The hall is free.")).toBeTruthy();
    expect(screen.getByText("Proposed edit to Groceries: waiting for the user in the chat that started this sub agent")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Apply" })).toBeNull();
    expect(screen.queryByLabelText("Ask AI")).toBeNull();
    expect(screen.queryByRole("button", { name: "Edit this message" })).toBeNull();
    s.close();
  });

  it("shows a decided proposal's outcome", async () => {
    agentThread.mockResolvedValue(session({ proposals: [{ event: "proposal", data: { ...PROPOSAL, thread: "01JSUB" }, status: "applied" }] }));
    const s = createChatSession({ agent: "lead" });
    render(() => <Conversation session={s} readOnly />);
    void s.openThread("01JSUB");
    expect(await screen.findByText("Proposed edit to Groceries: applied")).toBeTruthy();
    s.close();
  });

  it("streams a running session, with Stop", async () => {
    agentThread.mockResolvedValue(session({ running: true, turn: "turn-sub", started_ms: Date.now() }));
    const s = createChatSession({ agent: "lead" });
    render(() => <Conversation session={s} readOnly />);
    void s.openThread("01JSUB");
    await waitFor(() => expect(live).not.toBeNull());
    emit("text", { delta: "Still looking" });
    expect(await screen.findByText("Still looking")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(turns.aiTurnStop).toHaveBeenCalledWith("turn-sub");
    s.close();
  });
});
