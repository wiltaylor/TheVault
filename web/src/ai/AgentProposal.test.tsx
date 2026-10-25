import { cleanup, fireEvent, render, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";
import { turnsOver } from "./fakeTurns";

// The chat panel's agent proposal card (agents G20): the WCL diff; Apply sends it to POST
// /api/ai/apply and the sidebar's agent list reads again, an agent changed since the proposal is
// not applied, a refusal shows, Discard writes nothing, Apply all includes it, the outcome
// reaches the next message, and a change the AI made itself links to the agent's screen.

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
const putAgent = vi.fn();
const agents = vi.fn(async () => ({ agents: [], tools: [] }));
const aiModels = vi.fn();
const aiAccounts = vi.fn();
const app = vi.fn(async () => null);
const reloadDefinitions = vi.fn(async () => undefined);

const turns = turnsOver(streamEvents);

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    streamEvents,
    streamTurn: turns.streamTurn,
    api: { ...real.api, aiTurnStart: turns.aiTurnStart, aiTurnStop: turns.aiTurnStop, aiApply, putAgent, agents, aiModels, aiAccounts, app },
  };
});
vi.mock("../session", () => ({ reloadDefinitions }));

const { ApiError } = await import("../api");
const { ChatPanel } = await import("./ChatPanel");
const { newConversation, outcomesNow } = await import("./store");

const BEFORE = 'label = "Clerk"\n\npermissions {\n  types = ["memo"]\n}\n';
const AFTER = BEFORE.replace('"Clerk"', '"Filing clerk"');

const CHANGE = {
  id: "toolu_a",
  kind: "agent",
  op: "change",
  name: "clerk",
  label: "Clerk",
  exists: true,
  before: BEFORE,
  after: AFTER,
  issues: [],
  proposer: "assistant",
};

const DELETE = { id: "toolu_d", kind: "agent", op: "delete", name: "old", label: "Old", before: 'label = "Old"\n', proposer: "assistant" };

async function ask(m: ReturnType<typeof render>) {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Rename the clerk." } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
}

function propose(proposal: { id: string; name: string; op: string }) {
  const tool = proposal.op === "delete" ? "propose_agent_delete" : "propose_agent_change";
  live!.onEvent({ event: "tool", data: { id: proposal.id, name: tool, input: { name: proposal.name } } });
  live!.onEvent({ event: "agent_proposal", data: proposal });
  live!.onEvent({ event: "tool_result", data: { id: proposal.id, is_error: false, summary: { notes: [], agents: [proposal.name] } } });
}

beforeEach(() => {
  aiAccounts.mockResolvedValue({ default: "a1", accounts: [{ id: "a1", name: "Anthropic", kind: "anthropic", kind_label: "Anthropic API", model: "m", configured: true }] });
  aiModels.mockResolvedValue({ account: "a1", model: "m", models: [{ id: "m", display_name: "M" }] });
});

afterEach(() => {
  newConversation();
  live = null;
  cleanup();
  vi.clearAllMocks();
});

describe("agent proposals", () => {
  it("shows the WCL diff, applies on the server and reads the agent list again", async () => {
    aiApply.mockResolvedValue({ op: "agent_change", agent_name: "clerk" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to agent clerk" });
    expect(card).toHaveTextContent('- label = "Clerk"');
    expect(card).toHaveTextContent('+ label = "Filing clerk"');
    expect(within(card).getByRole("link", { name: "clerk" })).toHaveAttribute("href", "/agents/clerk");
    expect(m.getByText("Checked a change to agent clerk")).toBeInTheDocument();
    agents.mockClear();
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByText(/Applied\./)).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(CHANGE);
    expect(putAgent).not.toHaveBeenCalled();
    expect(within(card).getByRole("link", { name: "Open agent" })).toHaveAttribute("href", "/agents/clerk");
    await waitFor(() => expect(agents).toHaveBeenCalled());
    expect(outcomesNow()).toEqual(["[Applied by the user: changed agent clerk]"]);
  });

  it("does not apply an agent that changed since the proposal, and says so", async () => {
    aiApply.mockRejectedValue(new ApiError(409, { error: "stale", message: "the agent clerk changed since the proposal" }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to agent clerk" });
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByRole("alert")).toHaveTextContent("This agent changed since the proposal"));
    expect(within(card).queryByRole("button", { name: "Apply" })).toBeNull();
    expect(outcomesNow()[0]).toMatch(/^\[Failed to apply: change to agent clerk: This agent changed/);
  });

  it("shows why the server refused it", async () => {
    aiApply.mockRejectedValue(new ApiError(403, { error: "needs_admin", message: "Apply agent changes in the web app." }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to agent clerk" });
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByRole("alert")).toHaveTextContent("Apply agent changes in the web app."));
    expect(within(card).getByRole("button", { name: "Apply" })).toBeInTheDocument();
  });

  it("discards without writing", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to agent clerk" });
    fireEvent.click(within(card).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(within(card).getByText("Discarded.")).toBeInTheDocument());
    expect(aiApply).not.toHaveBeenCalled();
    expect(outcomesNow()).toEqual(["[Discarded by the user: change to agent clerk]"]);
  });

  it("shows a delete and a new agent, and Apply all applies both", async () => {
    aiApply.mockResolvedValueOnce({ op: "agent_create", agent_name: "helper" }).mockResolvedValueOnce({ op: "agent_delete", agent_name: "old" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    const create = { ...CHANGE, id: "toolu_n", op: "create", name: "helper", exists: false, before: null };
    propose(create);
    propose(DELETE);
    const created = await m.findByRole("region", { name: "Proposed new agent helper" });
    expect(created).toHaveTextContent('+ label = "Filing clerk"');
    const deleted = m.getByRole("region", { name: "Proposed delete of agent old" });
    expect(deleted).toHaveTextContent('- label = "Old"');
    fireEvent.click(m.getByRole("button", { name: "Apply all (2)" }));
    await waitFor(() => expect(aiApply).toHaveBeenCalledTimes(2));
    expect(aiApply).toHaveBeenNthCalledWith(1, create);
    expect(aiApply).toHaveBeenNthCalledWith(2, DELETE);
    await waitFor(() => expect(within(deleted).getByText(/Applied\./)).toBeInTheDocument());
    expect(outcomesNow()).toEqual(["[Applied by the user: created agent helper]", "[Applied by the user: deleted agent old]"]);
  });

  it("shows a change the AI made itself, linking to the agent", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    agents.mockClear();
    live!.onEvent({ event: "applied", data: { id: "toolu_a", op: "agent_create", agent_name: "helper" } });
    const card = await m.findByRole("region", { name: "Created agent helper" });
    expect(within(card).getByRole("link", { name: "helper" })).toHaveAttribute("href", "/agents/helper");
    await waitFor(() => expect(agents).toHaveBeenCalled());
  });
});
