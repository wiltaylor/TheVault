import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";

// The chat panel's type proposal card (ai D18): Apply writes through the type routes after
// reading the type again, Discard writes nothing, and a type changed since the proposal is not
// applied.

type OnEvent = (e: StreamEvent) => void;

let live: { onEvent: OnEvent; end: () => void } | null = null;

const streamEvents = vi.fn(
  (_path: string, _body: unknown, onEvent: OnEvent, signal?: AbortSignal) =>
    new Promise<void>((resolve, reject) => {
      live = { onEvent, end: resolve };
      signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
    }),
);
const readType = vi.fn();
const putType = vi.fn();
const changeType = vi.fn();
const aiModels = vi.fn();
const aiStatus = vi.fn();
const reloadDefinitions = vi.fn(async () => undefined);

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, streamEvents, api: { ...real.api, readType, putType, changeType, aiModels, aiStatus } };
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
  aiStatus.mockResolvedValue({ secrets_key: true, default_provider: "anthropic", providers: { anthropic: { configured: true, model: "m" } } });
  aiModels.mockResolvedValue({ provider: "anthropic", model: "m", models: [{ id: "m", display_name: "M" }] });
});

afterEach(() => {
  newConversation();
  live = null;
  cleanup();
  vi.clearAllMocks();
});

describe("type proposals", () => {
  it("shows the WCL diff and applies a change through the planned change route", async () => {
    readType.mockResolvedValue({ name: "memo", wcl: BEFORE });
    changeType.mockResolvedValueOnce({ plan: "token-1" }).mockResolvedValueOnce({ applied: true });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to type memo" });
    expect(card).toHaveTextContent('- tab "Main" {');
    expect(card).toHaveTextContent('+ tab "Details" {');
    expect(m.getByText("Checked a change to type memo")).toBeInTheDocument();
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Applied.")).toBeInTheDocument());
    expect(readType).toHaveBeenCalledWith("memo");
    expect(changeType).toHaveBeenNthCalledWith(1, "memo", AFTER, {});
    expect(changeType).toHaveBeenNthCalledWith(2, "memo", AFTER, {}, "token-1");
    expect(putType).not.toHaveBeenCalled();
    await waitFor(() => expect(reloadDefinitions).toHaveBeenCalled());
  });

  it("creates a new type with the type write route", async () => {
    readType.mockRejectedValue(new ApiError(400, { error: "unknown_type", message: "unknown type book" }));
    putType.mockResolvedValue({ name: "book", wcl: AFTER });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, { ...CHANGE, name: "book", exists: false, before: null });
    await m.findByRole("region", { name: "Proposed new type book" });
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Applied.")).toBeInTheDocument());
    expect(putType).toHaveBeenCalledWith("book", AFTER);
    expect(changeType).not.toHaveBeenCalled();
  });

  it("discards a type proposal without writing", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, CHANGE);
    await m.findByRole("region", { name: "Proposed change to type memo" });
    fireEvent.click(m.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(m.getByText("Discarded.")).toBeInTheDocument());
    expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
    expect(readType).not.toHaveBeenCalled();
    expect(putType).not.toHaveBeenCalled();
    expect(changeType).not.toHaveBeenCalled();
  });

  it("does not apply a type that changed since the proposal, and says so", async () => {
    readType.mockResolvedValue({ name: "memo", wcl: BEFORE.replace("Memo", "Memos") });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, CHANGE);
    await m.findByRole("region", { name: "Proposed change to type memo" });
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByRole("alert")).toHaveTextContent("This type changed since the proposal"));
    expect(changeType).not.toHaveBeenCalled();
    expect(putType).not.toHaveBeenCalled();
    expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
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
