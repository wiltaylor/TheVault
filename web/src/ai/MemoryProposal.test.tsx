import { cleanup, fireEvent, render, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";
import { turnsOver } from "./fakeTurns";

// The chat panel's memory proposal card (agents G21): a change to another agent's memories, its
// title and text lines; Apply sends it to POST /api/ai/apply and an open Memory tab reads again, a
// memory changed since the proposal is not applied, a refusal shows, Discard writes nothing, Apply
// all includes it, the outcome reaches the next message, and a change the AI made itself links to
// the agent's Memory tab.

type OnEvent = (e: StreamEvent) => void;

let live: { onEvent: OnEvent; end: () => void } | null = null;

const streamEvents = vi.fn(
  (_path: string, _body: unknown, onEvent: OnEvent, signal?: AbortSignal) =>
    new Promise<void>((resolve, reject) => {
      live = { onEvent, end: resolve };
      signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
    }),
);
const aiApply = vi.fn();
const agents = vi.fn(async () => ({ agents: [], tools: [] }));
const aiModels = vi.fn();
const aiProviders = vi.fn();
const app = vi.fn(async () => null);
const reloadDefinitions = vi.fn(async () => undefined);

const turns = turnsOver(streamEvents);

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    streamEvents,
    streamTurn: turns.streamTurn,
    api: { ...real.api, aiTurnStart: turns.aiTurnStart, aiTurnStop: turns.aiTurnStop, aiApply, agents, aiModels, aiProviders, app },
  };
});
vi.mock("../session", () => ({ reloadDefinitions }));

const { ApiError } = await import("../api");
const { ChatPanel } = await import("./ChatPanel");
const { newConversation, outcomesNow } = await import("./store");
const { memoriesVersion } = await import("../agents");

const CHANGE = {
  id: "toolu_a",
  kind: "agent_memory",
  op: "change",
  memory_agent: "clerk",
  agent_label: "Clerk",
  memory_id: "01JTEA",
  base_version: "v1",
  title: "Tea order",
  text: "Green tea.",
  before: { title: "Tea", text: "Earl Grey." },
  proposer: "assistant",
};

const CREATE = {
  ...CHANGE,
  id: "toolu_n",
  op: "create",
  memory_id: null,
  base_version: null,
  title: "Garden",
  text: "Water on Sundays.",
  before: null,
};

const DELETE = { ...CHANGE, id: "toolu_d", op: "delete", title: "Tea", text: "Earl Grey." };

async function ask(m: ReturnType<typeof render>) {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Tidy the clerk's memories." } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
}

function propose(proposal: { id: string; op: string; memory_id: string | null }) {
  const tool = proposal.op === "delete" ? "propose_agent_memory_delete" : "propose_agent_memory_change";
  const input = proposal.op === "delete" ? { id: proposal.memory_id } : { agent: "clerk", id: proposal.memory_id };
  live!.onEvent({ event: "tool", data: { id: proposal.id, name: tool, input } });
  live!.onEvent({ event: "agent_memory_proposal", data: proposal });
  live!.onEvent({ event: "tool_result", data: { id: proposal.id, is_error: false, summary: { notes: [] } } });
}

beforeEach(() => {
  aiProviders.mockResolvedValue({ default_provider: "anthropic", providers: { anthropic: { configured: true, model: "m" } } });
  aiModels.mockResolvedValue({ provider: "anthropic", model: "m", models: [{ id: "m", display_name: "M" }] });
});

afterEach(() => {
  newConversation();
  live = null;
  cleanup();
  vi.clearAllMocks();
});

describe("memory proposals", () => {
  it("shows the title and text before and after, and applies on the server", async () => {
    aiApply.mockResolvedValue({ op: "agent_memory_change", memory_id: "01JTEA", memory_agent: "clerk", version: "v2" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to a memory of Clerk" });
    expect(card).toHaveTextContent("- Earl Grey.");
    expect(card).toHaveTextContent("+ Green tea.");
    expect(card.querySelector("del")).toHaveTextContent("Tea");
    expect(within(card).getByText("Tea order")).toBeInTheDocument();
    expect(within(card).getByRole("link", { name: "Clerk" })).toHaveAttribute("href", "/agents/clerk/memory");
    expect(m.getByText("Checked a change to a memory of agent clerk")).toBeInTheDocument();
    const before = memoriesVersion();
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByText(/Applied\./)).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(CHANGE);
    expect(memoriesVersion()).toBeGreaterThan(before);
    expect(within(card).getByRole("link", { name: "Open memories" })).toHaveAttribute("href", "/agents/clerk/memory");
    expect(outcomesNow()).toEqual(["[Applied by the user: changed memory 01JTEA of agent clerk]"]);
  });

  it("does not apply a memory that changed since the proposal, and says so", async () => {
    aiApply.mockRejectedValue(new ApiError(409, { error: "stale", message: "the memory 01JTEA changed since the proposal" }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to a memory of Clerk" });
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByRole("alert")).toHaveTextContent("This memory changed since the proposal"));
    expect(within(card).queryByRole("button", { name: "Apply" })).toBeNull();
    expect(outcomesNow()[0]).toMatch(/^\[Failed to apply: change to memory 01JTEA of agent clerk: This memory changed/);
  });

  it("shows why the server refused it", async () => {
    aiApply.mockRejectedValue(new ApiError(403, { error: "not_permitted", message: "The memories of agent clerk are out of reach" }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to a memory of Clerk" });
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByRole("alert")).toHaveTextContent("out of reach"));
    expect(within(card).getByRole("button", { name: "Apply" })).toBeInTheDocument();
  });

  it("discards without writing", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to a memory of Clerk" });
    fireEvent.click(within(card).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(within(card).getByText("Discarded.")).toBeInTheDocument());
    expect(aiApply).not.toHaveBeenCalled();
    expect(outcomesNow()).toEqual(["[Discarded by the user: change to memory 01JTEA of agent clerk]"]);
  });

  it("shows a new memory and a forgotten one, and Apply all applies both", async () => {
    aiApply
      .mockResolvedValueOnce({ op: "agent_memory_create", memory_id: "01JNEW", memory_agent: "clerk", version: "v1" })
      .mockResolvedValueOnce({ op: "agent_memory_delete", memory_id: "01JTEA", memory_agent: "clerk" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CREATE);
    propose(DELETE);
    const created = await m.findByRole("region", { name: "Proposed new memory of Clerk" });
    expect(created).toHaveTextContent("+ Water on Sundays.");
    const deleted = m.getByRole("region", { name: "Proposed forgetting a memory of Clerk" });
    expect(deleted).toHaveTextContent("- Earl Grey.");
    fireEvent.click(m.getByRole("button", { name: "Apply all (2)" }));
    await waitFor(() => expect(aiApply).toHaveBeenCalledTimes(2));
    expect(aiApply).toHaveBeenNthCalledWith(1, CREATE);
    expect(aiApply).toHaveBeenNthCalledWith(2, DELETE);
    await waitFor(() => expect(within(deleted).getByText(/in the trash/)).toBeInTheDocument());
    expect(outcomesNow()).toEqual([
      '[Applied by the user: created memory 01JNEW "Garden" of agent clerk]',
      "[Applied by the user: forgot memory 01JTEA of agent clerk: moved to the trash]",
    ]);
  });

  it("shows a change the AI made itself, linking to the agent's memories", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    agents.mockClear();
    live!.onEvent({ event: "applied", data: { id: "toolu_a", op: "agent_memory_create", memory_id: "01JNEW", memory_agent: "clerk", version: "v1" } });
    const card = await m.findByRole("region", { name: "Added a memory of clerk" });
    expect(within(card).getByRole("link", { name: "clerk" })).toHaveAttribute("href", "/agents/clerk/memory");
    // A memory is a note: the agent list is not read again.
    expect(agents).not.toHaveBeenCalled();
  });
});
