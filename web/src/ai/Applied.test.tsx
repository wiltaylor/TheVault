import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";
import { turnsOver } from "./fakeTurns";

// The chat panel's applied cards (ai D22): a change the AI made itself, its confirmation off in
// Settings > AI, shows what was done and links to the note, the trash or the type, and open views
// of that note or type are told to refresh.

type OnEvent = (e: StreamEvent) => void;

let live: { onEvent: OnEvent; end: () => void } | null = null;

const streamEvents = vi.fn(
  (_path: string, _body: unknown, onEvent: OnEvent, signal?: AbortSignal) =>
    new Promise<void>((resolve, reject) => {
      live = { onEvent, end: resolve };
      signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
    }),
);
const aiModels = vi.fn();
const aiAccounts = vi.fn();
const app = vi.fn(async () => null);
const reloadDefinitions = vi.fn(async () => undefined);

const turns = turnsOver(streamEvents);

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, streamEvents, streamTurn: turns.streamTurn, api: { ...real.api, aiTurnStart: turns.aiTurnStart, aiTurnStop: turns.aiTurnStop, aiModels, aiAccounts, app } };
});
vi.mock("../session", () => ({ reloadDefinitions }));

const { ChatPanel } = await import("./ChatPanel");
const { newConversation } = await import("./store");
const { changes } = await import("../changes");

async function applied(m: ReturnType<typeof render>, data: Record<string, unknown>) {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Do it." } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
  live!.onEvent({ event: "tool", data: { id: data.id, name: "propose_x", input: {} } });
  live!.onEvent({ event: "applied", data });
  live!.onEvent({ event: "tool_result", data: { id: data.id, is_error: false, summary: { notes: [] } } });
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

const NOTE_CASES: [string, string, string][] = [
  ["update", "Edited Groceries", "Edited"],
  ["create", "Created Groceries", "Created"],
  ["archive", "Archived Groceries", "Archived"],
  ["unarchive", "Unarchived Groceries", "Unarchived"],
];

describe("applied cards", () => {
  for (const [op, name, words] of NOTE_CASES) {
    it(`says ${words} and links to the note for ${op}, and refreshes open notes`, async () => {
      const before = changes();
      const m = render(() => <ChatPanel noteId={() => ""} />);
      await applied(m, { id: "toolu_x", op, note_id: "01JNOTE", title: "Groceries", version: "v2" });
      const card = await m.findByRole("region", { name });
      expect(card).toHaveTextContent(words);
      expect(m.getByRole("link", { name: "Groceries" })).toHaveAttribute("href", "/notes/01JNOTE");
      await waitFor(() => expect(changes()).toBeGreaterThan(before));
      expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
    });
  }

  it("says a deleted note moved to the trash and links to the trash", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await applied(m, { id: "toolu_d", op: "delete", note_id: "01JSCRATCH", title: "Scratch", version: "v1" });
    const card = await m.findByRole("region", { name: "Moved Scratch to the trash" });
    expect(card).toHaveTextContent("where you can restore it");
    expect(m.getByRole("link", { name: "trash" })).toHaveAttribute("href", "/trash");
  });

  it("says a type changed or was created, links to it and reloads the definitions", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await applied(m, { id: "toolu_t", op: "type_change", type_name: "memo" });
    await m.findByRole("region", { name: "Changed type memo" });
    expect(m.getByRole("link", { name: "memo" })).toHaveAttribute("href", "/types/memo");
    await waitFor(() => expect(reloadDefinitions).toHaveBeenCalled());
    live!.onEvent({ event: "applied", data: { id: "toolu_n", op: "type_create", type_name: "book" } });
    await m.findByRole("region", { name: "Created type book" });
    expect(m.getByRole("link", { name: "book" })).toHaveAttribute("href", "/types/book");
  });

  it("names a note without a title by its id", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await applied(m, { id: "toolu_x", op: "update", note_id: "01JNOTE", title: "", version: "v2" });
    await m.findByRole("region", { name: "Edited 01JNOTE" });
  });
});
