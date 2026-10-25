import { cleanup, fireEvent, render, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";
import { turnsOver } from "./fakeTurns";

// The chat panel's taxonomy proposal card (ai D31): the WCL diff, the renames and the notes they
// move; Apply sends it to POST /api/ai/apply, a taxonomy changed since the proposal is not
// applied, Discard writes nothing, Apply all includes it, the outcome reaches the next message,
// and a change the AI made itself links to Settings > Taxonomies.

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
const putTaxonomy = vi.fn();
const aiModels = vi.fn();
const aiAccounts = vi.fn();
const app = vi.fn(async () => null);
const reloadDefinitions = vi.fn(async () => undefined);

const turns = turnsOver(streamEvents);

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, streamEvents, streamTurn: turns.streamTurn, api: { ...real.api, aiTurnStart: turns.aiTurnStart, aiTurnStop: turns.aiTurnStop, aiApply, putTaxonomy, aiModels, aiAccounts, app } };
});
vi.mock("../session", () => ({ reloadDefinitions }));

const { ApiError } = await import("../api");
const { ChatPanel } = await import("./ChatPanel");
const { newConversation, outcomesNow } = await import("./store");

const BEFORE = 'label = "Stage"\n\nterm "Todo"\n\nterm "Done"\n';
const AFTER = BEFORE.replace('"Todo"', '"Open"');

const CHANGE = {
  id: "toolu_t",
  kind: "taxonomy",
  op: "change",
  name: "stage",
  exists: true,
  before: BEFORE,
  after: AFTER,
  renames: [{ from: "Todo", to: "Open" }],
  issues: [],
  affected: 3,
};

const DELETE = { id: "toolu_d", kind: "taxonomy", op: "delete", name: "shade", before: 'label = "Shade"\n\nterm "Dark"\n' };

async function ask(m: ReturnType<typeof render>) {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Call Todo Open." } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
}

function propose(proposal: { id: string; name: string; op: string }) {
  const tool = proposal.op === "delete" ? "propose_taxonomy_delete" : "propose_taxonomy_change";
  live!.onEvent({ event: "tool", data: { id: proposal.id, name: tool, input: { name: proposal.name } } });
  live!.onEvent({ event: "taxonomy_proposal", data: proposal });
  live!.onEvent({ event: "tool_result", data: { id: proposal.id, is_error: false, summary: { notes: [], taxonomies: [proposal.name] } } });
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

describe("taxonomy proposals", () => {
  it("shows the WCL diff and the renames, and applies on the server", async () => {
    aiApply.mockResolvedValue({ op: "taxonomy_change", taxonomy_name: "stage", notes: 3 });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to taxonomy stage" });
    expect(card).toHaveTextContent('- term "Todo"');
    expect(card).toHaveTextContent('+ term "Open"');
    expect(within(card).getByRole("list", { name: "Renamed terms" })).toHaveTextContent("Todo → Open");
    expect(card).toHaveTextContent("Moves 3 notes to the new terms.");
    expect(within(card).getByRole("link", { name: "stage" })).toHaveAttribute("href", "/settings/taxonomies/stage");
    expect(m.getByText("Checked a change to taxonomy stage")).toBeInTheDocument();
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByText(/Applied\./)).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(CHANGE);
    expect(putTaxonomy).not.toHaveBeenCalled();
    await waitFor(() => expect(reloadDefinitions).toHaveBeenCalled());
    expect(outcomesNow()).toEqual(["[Applied by the user: changed taxonomy stage, its renames moving 3 notes]"]);
  });

  it("does not apply a taxonomy that changed since the proposal, and says so", async () => {
    aiApply.mockRejectedValue(new ApiError(409, { error: "stale", message: "the taxonomy stage changed since the proposal" }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to taxonomy stage" });
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByRole("alert")).toHaveTextContent("This taxonomy changed since the proposal"));
    expect(within(card).queryByRole("button", { name: "Apply" })).toBeNull();
    expect(reloadDefinitions).not.toHaveBeenCalled();
    expect(outcomesNow()[0]).toMatch(/^\[Failed to apply: change to taxonomy stage: This taxonomy changed/);
  });

  it("says to apply it in the web app when the caller lacks admin", async () => {
    aiApply.mockRejectedValue(new ApiError(403, { error: "needs_admin", message: "Apply taxonomy changes in the web app." }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to taxonomy stage" });
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByRole("alert")).toHaveTextContent("Apply taxonomy changes in the web app."));
    expect(within(card).getByRole("button", { name: "Apply" })).toBeInTheDocument();
  });

  it("discards without writing", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to taxonomy stage" });
    fireEvent.click(within(card).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(within(card).getByText("Discarded.")).toBeInTheDocument());
    expect(aiApply).not.toHaveBeenCalled();
    expect(outcomesNow()).toEqual(["[Discarded by the user: change to taxonomy stage]"]);
  });

  it("shows a delete and a new taxonomy, and Apply all applies both", async () => {
    aiApply
      .mockResolvedValueOnce({ op: "taxonomy_create", taxonomy_name: "size", notes: 0 })
      .mockResolvedValueOnce({ op: "taxonomy_delete", taxonomy_name: "shade", notes: 0 });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    const create = { ...CHANGE, id: "toolu_n", op: "create", name: "size", exists: false, before: null, renames: [], affected: 0 };
    propose(create);
    propose(DELETE);
    const created = await m.findByRole("region", { name: "Proposed new taxonomy size" });
    expect(created).toHaveTextContent('+ term "Open"');
    expect(within(created).queryByRole("list", { name: "Renamed terms" })).toBeNull();
    const deleted = m.getByRole("region", { name: "Proposed delete of taxonomy shade" });
    expect(deleted).toHaveTextContent('- term "Dark"');
    fireEvent.click(m.getByRole("button", { name: "Apply all (2)" }));
    await waitFor(() => expect(aiApply).toHaveBeenCalledTimes(2));
    expect(aiApply).toHaveBeenNthCalledWith(1, create);
    expect(aiApply).toHaveBeenNthCalledWith(2, DELETE);
    await waitFor(() => expect(within(deleted).getByText(/Applied\./)).toBeInTheDocument());
    expect(outcomesNow()).toEqual(["[Applied by the user: created taxonomy size]", "[Applied by the user: deleted taxonomy shade]"]);
  });

  it("shows a change the AI made itself, linking to Settings > Taxonomies", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    live!.onEvent({ event: "applied", data: { id: "toolu_t", op: "taxonomy_change", taxonomy_name: "stage", notes: 1 } });
    const card = await m.findByRole("region", { name: "Changed taxonomy stage" });
    expect(within(card).getByRole("link", { name: "stage" })).toHaveAttribute("href", "/settings/taxonomies/stage");
    await waitFor(() => expect(reloadDefinitions).toHaveBeenCalled());
  });
});
