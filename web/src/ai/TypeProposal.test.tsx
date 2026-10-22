import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";

// The chat panel's type proposal card (ai D18): Apply sends it to POST /api/ai/apply, which
// applies it on the home server (ai D23), Discard writes nothing, a type changed since the
// proposal is not applied, and without admin the card says to apply it in the web app.

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
const putType = vi.fn();
const changeType = vi.fn();
const aiModels = vi.fn();
const aiProviders = vi.fn();
const app = vi.fn(async () => null);
const reloadDefinitions = vi.fn(async () => undefined);

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, streamEvents, api: { ...real.api, aiApply, putType, changeType, aiModels, aiProviders, app } };
});
vi.mock("../session", () => ({ reloadDefinitions }));

const { ApiError } = await import("../api");
const { ChatPanel } = await import("./ChatPanel");
const { newConversation } = await import("./store");
const { wclDiff } = await import("./TypeProposalCard");

const BEFORE = 'label = "Memo"\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n  }\n}\n';
const AFTER = BEFORE.replace('tab "Main"', 'tab "Details"');

const CHANGE = { id: "toolu_t", name: "memo", exists: true, before: BEFORE, after: AFTER, issues: [] };

async function propose(m: ReturnType<typeof render>, proposal: unknown) {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Rename the tab." } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
  live!.onEvent({ event: "tool", data: { id: "toolu_t", name: "propose_type_change", input: { name: "memo", wcl: AFTER } } });
  live!.onEvent({ event: "type_proposal", data: proposal });
  live!.onEvent({ event: "tool_result", data: { id: "toolu_t", is_error: false, summary: { notes: [], types: ["memo"] } } });
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

describe("type proposals", () => {
  it("shows the WCL diff and applies a change on the server", async () => {
    aiApply.mockResolvedValue({ op: "type_change", type_name: "memo" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to type memo" });
    expect(card).toHaveTextContent('- tab "Main" {');
    expect(card).toHaveTextContent('+ tab "Details" {');
    expect(m.getByText("Checked a change to type memo")).toBeInTheDocument();
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Applied.")).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(CHANGE);
    expect(changeType).not.toHaveBeenCalled();
    expect(putType).not.toHaveBeenCalled();
    await waitFor(() => expect(reloadDefinitions).toHaveBeenCalled());
  });

  it("creates a new type on the server", async () => {
    aiApply.mockResolvedValue({ op: "type_create", type_name: "book" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    const book = { ...CHANGE, name: "book", exists: false, before: null };
    await propose(m, book);
    await m.findByRole("region", { name: "Proposed new type book" });
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Applied.")).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(book);
  });

  it("discards a type proposal without writing", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, CHANGE);
    await m.findByRole("region", { name: "Proposed change to type memo" });
    fireEvent.click(m.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(m.getByText("Discarded.")).toBeInTheDocument());
    expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
    expect(aiApply).not.toHaveBeenCalled();
  });

  it("does not apply a type that changed since the proposal, and says so", async () => {
    aiApply.mockRejectedValue(new ApiError(409, { error: "stale", message: "the type memo changed since the proposal" }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, CHANGE);
    await m.findByRole("region", { name: "Proposed change to type memo" });
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByRole("alert")).toHaveTextContent("This type changed since the proposal"));
    expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
  });

  it("says to apply a type change in the web app when the caller lacks admin", async () => {
    aiApply.mockRejectedValue(new ApiError(403, { error: "needs_admin", message: "Apply type changes in the web app." }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, CHANGE);
    await m.findByRole("region", { name: "Proposed change to type memo" });
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByRole("alert")).toHaveTextContent("Apply type changes in the web app."));
    expect(reloadDefinitions).not.toHaveBeenCalled();
  });

  it("folds long unchanged runs in the WCL diff", () => {
    const before = ["a", "b", "c", "d", "e", "f", "g"].join("\n");
    const after = before.replace("g", "G");
    expect(wclDiff(before, after)).toEqual([
      { type: "skip", count: 4 },
      { type: "same", text: "e" },
      { type: "same", text: "f" },
      { type: "del", text: "g" },
      { type: "add", text: "G" },
    ]);
  });
});
