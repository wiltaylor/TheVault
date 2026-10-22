import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";

// The chat panel's create, archive, unarchive and delete proposals (ai D20): Apply calls the
// notes route for each, an archive or delete reads the note again first and does nothing to a
// note that changed, and Apply all applies a turn's proposals in order, stopping at the first
// failure.

type OnEvent = (e: StreamEvent) => void;

let live: { onEvent: OnEvent; end: () => void } | null = null;

const streamEvents = vi.fn(
  (_path: string, _body: unknown, onEvent: OnEvent, signal?: AbortSignal) =>
    new Promise<void>((resolve, reject) => {
      live = { onEvent, end: resolve };
      signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
    }),
);
const note = vi.fn();
const createNote = vi.fn();
const updateNote = vi.fn();
const archiveNote = vi.fn();
const unarchiveNote = vi.fn();
const deleteNote = vi.fn();
const aiModels = vi.fn();
const aiStatus = vi.fn();

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    streamEvents,
    api: { ...real.api, note, createNote, updateNote, archiveNote, unarchiveNote, deleteNote, aiModels, aiStatus },
  };
});

const { ApiError } = await import("../api");
const { ChatPanel } = await import("./ChatPanel");
const { newConversation, NOTE_CHANGED } = await import("./store");

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

/** A note as GET /api/notes/{id} answers it. */
function stored(id: string, version: string, archived = false) {
  return { id, type: "memo", version, wcl: "", note: { fields: { title: "T", ...(archived ? { archived: true } : {}) }, blocks: [] } };
}

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
  aiStatus.mockResolvedValue({ secrets_key: true, default_provider: "anthropic", providers: { anthropic: { configured: true, model: "m" } } });
  aiModels.mockResolvedValue({ provider: "anthropic", model: "m", models: [{ id: "m", display_name: "M" }] });
});

afterEach(() => {
  newConversation();
  live = null;
  cleanup();
  vi.clearAllMocks();
});

describe("note proposals", () => {
  it("creates a proposed note through the create route and links to it", async () => {
    createNote.mockResolvedValue(stored("01JNEW", "v1"));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, CREATE);
    const card = await m.findByRole("region", { name: "Proposed new note Reading list" });
    expect(card).toHaveTextContent("Dune");
    expect(card).toHaveTextContent("(memo)");
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Created.", { exact: false })).toBeInTheDocument());
    expect(createNote).toHaveBeenCalledWith("memo", DOC);
    expect(m.getByRole("link", { name: "Open the new note" })).toHaveAttribute("href", "/notes/01JNEW");
  });

  it("archives through the archive route at the note's current version", async () => {
    note.mockResolvedValue(stored("01JOLD", "v2"));
    archiveNote.mockResolvedValue(stored("01JOLD", "v3", true));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, ARCHIVE);
    await m.findByRole("region", { name: "Proposed archive of Old plan" });
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Archived.")).toBeInTheDocument());
    expect(note).toHaveBeenCalledWith("01JOLD");
    expect(archiveNote).toHaveBeenCalledWith("01JOLD", "v2");
    expect(unarchiveNote).not.toHaveBeenCalled();
  });

  it("unarchives through the unarchive route", async () => {
    note.mockResolvedValue(stored("01JOLD", "v2", true));
    unarchiveNote.mockResolvedValue(stored("01JOLD", "v3"));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, UNARCHIVE);
    await m.findByRole("region", { name: "Proposed unarchive of Old plan" });
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Unarchived.")).toBeInTheDocument());
    expect(unarchiveNote).toHaveBeenCalledWith("01JOLD", "v2");
    expect(archiveNote).not.toHaveBeenCalled();
  });

  it("does not archive a note that is already archived, and says so", async () => {
    note.mockResolvedValue(stored("01JOLD", "v2", true));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, ARCHIVE);
    await m.findByRole("region", { name: "Proposed archive of Old plan" });
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByRole("alert")).toHaveTextContent("already archived"));
    expect(archiveNote).not.toHaveBeenCalled();
    expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
  });

  it("deletes through the delete route, moving the note to the trash", async () => {
    note.mockResolvedValue(stored("01JSCRATCH", "v1"));
    deleteNote.mockResolvedValue({});
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, DELETE);
    const card = await m.findByRole("region", { name: "Proposed delete of Scratch" });
    expect(card).toHaveTextContent("moves the note to the trash");
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Moved to the trash.", { exact: false })).toBeInTheDocument());
    expect(deleteNote).toHaveBeenCalledWith("01JSCRATCH", "v1");
    expect(m.getByRole("link", { name: "Open the trash" })).toHaveAttribute("href", "/trash");
  });

  it("does not delete a note changed since the proposal", async () => {
    note.mockResolvedValue(stored("01JSCRATCH", "v2"));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, DELETE);
    await m.findByRole("region", { name: "Proposed delete of Scratch" });
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByRole("alert")).toHaveTextContent(NOTE_CHANGED));
    expect(deleteNote).not.toHaveBeenCalled();
    expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
    fireEvent.click(m.getByRole("button", { name: "Discard" }));
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
    updateNote.mockImplementation(async () => void calls.push("update"));
    note.mockImplementation(async (id: string) => stored(id, "v1"));
    archiveNote.mockImplementation(async () => {
      calls.push("archive");
      throw new ApiError(500, { error: "internal", message: "The disk is full." });
    });
    deleteNote.mockImplementation(async () => void calls.push("delete"));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await propose(m, UPDATE, ARCHIVE, DELETE);
    fireEvent.click(await m.findByRole("button", { name: "Apply all (3)" }));
    await waitFor(() => expect(m.getByRole("alert")).toHaveTextContent("The disk is full."));
    expect(calls).toEqual(["update", "archive"]);
    expect(deleteNote).not.toHaveBeenCalled();
    expect(m.getByRole("region", { name: "Proposed edit to Groceries" })).toHaveTextContent("Applied.");
    const del = m.getByRole("region", { name: "Proposed delete of Scratch" });
    expect(del.querySelector("button.primary")).toHaveTextContent("Apply");
    // The failed archive and the delete are still pending.
    expect(m.getByRole("button", { name: "Apply all (2)" })).toBeInTheDocument();
  });
});
