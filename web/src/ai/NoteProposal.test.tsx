import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";

// The chat panel's create, archive, unarchive and delete proposals (ai D20): Apply sends each to
// POST /api/ai/apply as the chat sent it, which applies it on the home server at the version it
// was proposed on (ai D23); a note that changed since is a conflict the card shows, and Apply all
// applies a turn's proposals in order, stopping at the first failure.

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
const aiModels = vi.fn();
const aiProviders = vi.fn();
const app = vi.fn(async () => null);
const updateNote = vi.fn();
const deleteNote = vi.fn();

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    streamEvents,
    api: { ...real.api, aiApply, aiModels, aiProviders, app, updateNote, deleteNote },
  };
});

const { ApiError } = await import("../api");
const { ChatPanel } = await import("./ChatPanel");
const { newConversation, NOTE_CHANGED, NOTE_GONE } = await import("./store");

const DOC = { fields: { title: "Reading list" }, blocks: [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text: "Dune" } }] }] };

const CREATE = { id: "toolu_c", op: "create", type: "memo", title: "Reading list", after: DOC };
const ARCHIVE = { id: "toolu_a", op: "archive", note_id: "01JOLD", title: "Old plan", base_version: "v1" };
const UNARCHIVE = { ...ARCHIVE, id: "toolu_u", op: "unarchive" };
const DELETE = { id: "toolu_d", op: "delete", note_id: "01JSCRATCH", title: "Scratch", base_version: "v1" };
const UPDATE = {
  id: "toolu_e",
  op: "update",
  note_id: "01JEDIT",
  title: "Groceries",
  base_version: "v1",
  before: { fields: { title: "Groceries" }, blocks: [] },
  after: { fields: { title: "Groceries and more" }, blocks: [] },
  ops: [{ op: "field", name: "title", before: "Groceries", after: "Groceries and more" }],
};

const conflict = () => new ApiError(409, { error: "conflict", message: "conflict: the note changed" });

async function propose(m: ReturnType<typeof render>, ...proposals: { id: string }[]) {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Tidy my notes." } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
  for (const p of proposals) {
    live!.onEvent({ event: "tool", data: { id: p.id, name: "propose_x", input: {} } });
    live!.onEvent({ event: "proposal", data: p });
    live!.onEvent({ event: "tool_result", data: { id: p.id, is_error: false, summary: { notes: [] } } });
  }
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

describe("note proposals", () => {
  it("creates a proposed note on the server and links to it", async () => {
    aiApply.mockResolvedValue({ op: "create", note_id: "01JNEW", version: "v1" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, CREATE);
    const card = await m.findByRole("region", { name: "Proposed new note Reading list" });
    expect(card).toHaveTextContent("Dune");
    expect(card).toHaveTextContent("(memo)");
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Created.", { exact: false })).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(CREATE);
    expect(m.getByRole("link", { name: "Open the new note" })).toHaveAttribute("href", "/notes/01JNEW");
  });

  it("archives and unarchives at the version proposed", async () => {
    aiApply.mockImplementation(async (p: { op: string; note_id: string }) => ({ op: p.op, note_id: p.note_id, version: "v2" }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, ARCHIVE, UNARCHIVE);
    await m.findByRole("region", { name: "Proposed archive of Old plan" });
    fireEvent.click(m.getAllByRole("button", { name: "Apply" })[0]!);
    await waitFor(() => expect(m.getByText("Archived.")).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(ARCHIVE);
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Unarchived.")).toBeInTheDocument());
    expect(aiApply).toHaveBeenLastCalledWith(UNARCHIVE);
  });

  it("does not archive a note that changed since the proposal, and says so", async () => {
    aiApply.mockRejectedValue(conflict());
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, ARCHIVE);
    await m.findByRole("region", { name: "Proposed archive of Old plan" });
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByRole("alert")).toHaveTextContent(NOTE_CHANGED));
    expect(NOTE_CHANGED).toContain("This note changed since the proposal");
    expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
  });

  it("deletes on the server, moving the note to the trash", async () => {
    aiApply.mockResolvedValue({ op: "delete", note_id: "01JSCRATCH", version: "v1" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, DELETE);
    const card = await m.findByRole("region", { name: "Proposed delete of Scratch" });
    expect(card).toHaveTextContent("moves the note to the trash");
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Moved to the trash.", { exact: false })).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(DELETE);
    expect(deleteNote).not.toHaveBeenCalled();
    expect(m.getByRole("link", { name: "Open the trash" })).toHaveAttribute("href", "/trash");
  });

  it("does not delete a note that changed or went since the proposal", async () => {
    aiApply.mockRejectedValueOnce(conflict()).mockRejectedValueOnce(new ApiError(404, { error: "not_found", message: "no such note" }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, DELETE, { ...DELETE, id: "toolu_d2" });
    await m.findAllByRole("region", { name: "Proposed delete of Scratch" });
    fireEvent.click(m.getAllByRole("button", { name: "Apply" })[0]!);
    await waitFor(() => expect(m.getByRole("alert")).toHaveTextContent(NOTE_CHANGED));
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getAllByRole("alert")[1]).toHaveTextContent(NOTE_GONE));
    expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
    fireEvent.click(m.getAllByRole("button", { name: "Discard" })[0]!);
    await waitFor(() => expect(m.getByText("Discarded.")).toBeInTheDocument());
  });

  it("shows Apply all only for two or more pending proposals", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, CREATE);
    await m.findByRole("region", { name: "Proposed new note Reading list" });
    expect(m.queryByRole("button", { name: /Apply all/ })).toBeNull();
    live!.onEvent({ event: "proposal", data: DELETE });
    await waitFor(() => expect(m.getByRole("button", { name: "Apply all (2)" })).toBeInTheDocument());
    fireEvent.click(m.getAllByRole("button", { name: "Discard" })[0]!);
    await waitFor(() => expect(m.queryByRole("button", { name: /Apply all/ })).toBeNull());
  });

  it("applies all in order and stops at the first failure", async () => {
    const calls: string[] = [];
    aiApply.mockImplementation(async (p: { op: string; note_id: string }) => {
      calls.push(p.op);
      if (p.op === "archive") throw new ApiError(500, { error: "internal", message: "The disk is full." });
      return { op: p.op, note_id: p.note_id, version: "v2" };
    });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, UPDATE, ARCHIVE, DELETE);
    fireEvent.click(await m.findByRole("button", { name: "Apply all (3)" }));
    await waitFor(() => expect(m.getByRole("alert")).toHaveTextContent("The disk is full."));
    expect(calls).toEqual(["update", "archive"]);
    expect(updateNote).not.toHaveBeenCalled();
    expect(m.getByRole("region", { name: "Proposed edit to Groceries" })).toHaveTextContent("Applied.");
    const del = m.getByRole("region", { name: "Proposed delete of Scratch" });
    expect(del.querySelector("button.primary")).toHaveTextContent("Apply");
    // The failed archive and the delete are still pending.
    expect(m.getByRole("button", { name: "Apply all (2)" })).toBeInTheDocument();
  });
});
