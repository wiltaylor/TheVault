import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";

type OnEvent = (e: StreamEvent) => void;

/** The turn in flight: the panel's event callback, its abort signal, and a way to end it. */
let live: { onEvent: OnEvent; signal: AbortSignal; body: unknown; end: () => void } | null = null;

const streamEvents = vi.fn(
  (_path: string, body: unknown, onEvent: OnEvent, signal?: AbortSignal) =>
    new Promise<void>((resolve, reject) => {
      live = { onEvent, signal: signal!, body, end: resolve };
      signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
    }),
);
const updateNote = vi.fn();
const aiApply = vi.fn();
const aiModels = vi.fn();
const aiProviders = vi.fn();
const app = vi.fn();
const aiTurnStart = vi.fn();
const aiTurnPoll = vi.fn();
const aiTurnStop = vi.fn();
const appSync = vi.fn();
const appSyncNow = vi.fn();

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    streamEvents,
    api: { ...real.api, updateNote, aiApply, aiModels, aiProviders, app, aiTurnStart, aiTurnPoll, aiTurnStop, appSync, appSyncNow },
  };
});

const { ChatPanel } = await import("./ChatPanel");
const { newConversation, transcriptNow, forgetApp } = await import("./store");
const { forgetAppInfo } = await import("../inApp");
const { generalActions } = await import("../palette/actions");
const { changes, onNoteChangedElsewhere } = await import("../changes");
const { parseBlocks, parseInline } = await import("./markdown");
const { blockLines, lineDiff } = await import("./diff");

const STATUS = {
  default_provider: "anthropic",
  providers: {
    anthropic: { configured: true, model: "claude-opus-5" },
    openai: { configured: false, model: null },
    chatgpt: { configured: false, model: null },
  },
  confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true },
};

const MODELS = {
  provider: "anthropic",
  model: "claude-opus-5",
  models: [
    { id: "claude-opus-5", display_name: "Claude Opus 5" },
    { id: "claude-sonnet-5", display_name: "Claude Sonnet 5" },
  ],
};

const PROPOSAL = {
  id: "toolu_p",
  note_id: "01JNOTE",
  title: "Groceries",
  base_version: "v1",
  before: { fields: { title: "Groceries" }, blocks: [] },
  after: { fields: { title: "Groceries and more" }, blocks: [] },
  ops: [{ op: "field", name: "title", before: "Groceries", after: "Groceries and more" }],
};

function emit(event: string, data: unknown) {
  live!.onEvent({ event, data });
}

async function askIt(m: ReturnType<typeof render>, text = "What is on my list?") {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: text } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
}

beforeEach(() => {
  app.mockResolvedValue(null);
  aiProviders.mockResolvedValue(STATUS);
  aiModels.mockResolvedValue(MODELS);
});

afterEach(() => {
  newConversation();
  live = null;
  cleanup();
  vi.clearAllMocks();
  forgetAppInfo();
  forgetApp();
});

describe("ChatPanel", () => {
  it("renders streamed text as it arrives and keeps the turn's transcript", async () => {
    const m = render(() => <ChatPanel noteId={() => "01JOPEN"} />);
    await waitFor(() => expect(m.getByLabelText("Model")).toHaveValue("claude-opus-5"));
    await askIt(m);
    expect(live!.body).toMatchObject({ provider: "anthropic", model: "claude-opus-5", note_id: "01JOPEN", messages: [{ role: "user" }] });
    emit("text", { delta: "You need " });
    await waitFor(() => expect(m.getByRole("list", { name: "Conversation" })).toHaveTextContent("You need"));
    expect(m.getByRole("list", { name: "Conversation" })).not.toHaveTextContent("milk");
    emit("text", { delta: "**milk**." });
    await waitFor(() => expect(m.getByText("milk").tagName).toBe("STRONG"));
    const answer = { role: "assistant", content: [{ type: "text", text: "You need **milk**." }] };
    emit("done", { stop: "end_turn", messages: [answer] });
    live!.end();
    await waitFor(() => expect(m.getByRole("button", { name: "Send" })).toBeInTheDocument());
    expect(transcriptNow()).toHaveLength(2);
    expect(transcriptNow()[1]).toEqual(answer);
  });

  it("shows tool steps with links to the notes they returned", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m);
    emit("tool", { id: "t1", name: "search_notes", input: { text: "list", limit: 5 } });
    emit("tool_result", { id: "t1", is_error: false, summary: { notes: [{ id: "01JA", title: "Shopping list" }] } });
    await waitFor(() => expect(m.getByText("Searched for “list”")).toBeInTheDocument());
    expect(m.getByRole("link", { name: "Shopping list" })).toHaveAttribute("href", "/notes/01JA");
  });

  it("shows a web_search step with its results linking out", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m);
    emit("tool", { id: "w1", name: "web_search", input: { query: "slate roof", count: 5 } });
    emit("tool_result", {
      id: "w1",
      is_error: false,
      summary: {
        notes: [],
        links: [
          { title: "Slate roofs", url: "https://example.com/slate" },
          { title: "Bad", url: "javascript:alert(1)" },
        ],
      },
    });
    await waitFor(() => expect(m.getByText("Searched the web for “slate roof”")).toBeInTheDocument());
    expect(m.getByText("(1 found)")).toBeInTheDocument();
    const link = m.getByRole("link", { name: "Slate roofs" });
    expect(link).toHaveAttribute("href", "https://example.com/slate");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
    expect(m.queryByRole("link", { name: "Bad" })).toBeNull();
  });

  it("shows a read_asset step's image as a thumbnail from the asset's URL", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m);
    emit("tool", { id: "t2", name: "read_asset", input: { id: "01JIMG" } });
    emit("tool_result", {
      id: "t2",
      is_error: false,
      summary: { notes: [], asset: { id: "01JIMG", filename: "roof.png", media_type: "image/png", size: 1200, kind: "image" } },
    });
    const img = await m.findByRole("img", { name: "roof.png" });
    expect(img).toHaveAttribute("src", "/api/assets/01JIMG");
    expect(m.getByText("Read a file")).toBeInTheDocument();
    expect(m.getByText("(roof.png)")).toBeInTheDocument();
    // A PDF step names the file and shows no image.
    emit("tool", { id: "t3", name: "read_asset", input: { id: "01JPDF" } });
    emit("tool_result", {
      id: "t3",
      is_error: false,
      summary: { notes: [], asset: { id: "01JPDF", filename: "report.pdf", media_type: "application/pdf", size: 9000, kind: "pdf" } },
    });
    await waitFor(() => expect(m.getByText("(report.pdf)")).toBeInTheDocument());
    expect(m.getAllByRole("img")).toHaveLength(1);
  });

  it("applies a proposal on the server with POST /api/ai/apply, as the chat sent it", async () => {
    aiApply.mockResolvedValue({ op: "update", note_id: "01JNOTE", version: "v2" });
    const before = changes();
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m);
    emit("proposal", PROPOSAL);
    const card = await m.findByRole("region", { name: "Proposed edit to Groceries" });
    expect(card).toHaveTextContent("Groceries and more");
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Applied.")).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith({ ...PROPOSAL, op: "update" });
    expect(updateNote).not.toHaveBeenCalled();
    // The web app asks for no sync round: the server's notes are its own.
    await waitFor(() => expect(changes()).toBeGreaterThan(before));
    expect(appSyncNow).not.toHaveBeenCalled();
  });

  it("discards a proposal without writing", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m);
    emit("proposal", PROPOSAL);
    await m.findByRole("region", { name: "Proposed edit to Groceries" });
    fireEvent.click(m.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(m.getByText("Discarded.")).toBeInTheDocument());
    expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
    expect(updateNote).not.toHaveBeenCalled();
  });

  it("shows a refusal with the model and category", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m);
    emit("text", { delta: "Partial" });
    emit("refusal", {
      provider: "anthropic",
      model: "claude-opus-5",
      category: "cyber",
      explanation: null,
      messages: [{ role: "assistant", content: [{ type: "text", text: "Partial" }] }],
    });
    live!.end();
    const alert = await m.findByRole("alert");
    expect(alert).toHaveTextContent("Anthropic claude-opus-5 refused this request (category: cyber)");
    expect(m.getByRole("list", { name: "Conversation" })).toHaveTextContent("Partial");
  });

  it("stops the turn by aborting the request", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m);
    emit("text", { delta: "Thinking" });
    fireEvent.click(m.getByRole("button", { name: "Stop" }));
    expect(live!.signal.aborted).toBe(true);
    await waitFor(() => expect(m.getByText("Stopped.")).toBeInTheDocument());
    expect(m.getByRole("button", { name: "Send" })).toBeInTheDocument();
  });

  it("starts a new conversation", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m);
    emit("done", { stop: "end_turn", messages: [] });
    live!.end();
    await waitFor(() => expect(m.getByRole("button", { name: "Send" })).toBeInTheDocument());
    fireEvent.click(m.getByRole("button", { name: "New conversation" }));
    expect(transcriptNow()).toHaveLength(0);
    expect(m.getByRole("list", { name: "Conversation" })).toBeEmptyDOMElement();
  });

  it("offers the configured providers and the default model", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await waitFor(() => expect(m.getByLabelText("Model")).toHaveValue("claude-opus-5"));
    expect((m.getByRole("option", { name: "OpenAI" }) as HTMLOptionElement).disabled).toBe(true);
  });
});

/** A turn's events as the embedded server hands them to the webview, in two polls. */
function polls(events: StreamEvent[]) {
  const half = Math.ceil(events.length / 2);
  aiTurnPoll
    .mockResolvedValueOnce({ events: events.slice(0, half), next: half, done: false })
    .mockResolvedValueOnce({ events: events.slice(half), next: events.length, done: true });
}

const APP = { app: true, paired: true, device_id: "d1", device_name: "Laptop", server_url: "https://home.example" };

describe("ChatPanel in the desktop and Android apps", () => {
  beforeEach(() => {
    app.mockResolvedValue(APP);
    aiTurnStart.mockResolvedValue("turn-1");
    aiTurnStop.mockResolvedValue(undefined);
  });

  it("is offered in every client: Ask AI is in the palette", () => {
    const ids = generalActions({ phone: false, dark: false, androidApp: true }).map((a) => a.id);
    expect(ids).toContain("ai:ask");
  });

  it("runs a turn in the turns form, polling the embedded server for its events", async () => {
    const answer = { role: "assistant", content: [{ type: "text", text: "Hello there." }] };
    polls([
      { event: "text", data: { delta: "Hello " } },
      { event: "text", data: { delta: "there." } },
      { event: "done", data: { stop: "end_turn", messages: [answer] } },
    ]);
    const m = render(() => <ChatPanel noteId={() => "01JOPEN"} />);
    await waitFor(() => expect(m.getByLabelText("Model")).toHaveValue("claude-opus-5"));
    fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Hi" } });
    fireEvent.click(m.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(m.getByRole("list", { name: "Conversation" })).toHaveTextContent("Hello there."));
    await waitFor(() => expect(m.getByRole("button", { name: "Send" })).toBeInTheDocument());
    expect(streamEvents).not.toHaveBeenCalled();
    expect(aiTurnStart).toHaveBeenCalledWith(expect.objectContaining({ provider: "anthropic", note_id: "01JOPEN", messages: [expect.objectContaining({ role: "user" })] }));
    expect(aiTurnPoll.mock.calls.map((c) => c.slice(0, 2))).toEqual([
      ["turn-1", 0],
      ["turn-1", 2],
    ]);
    expect(transcriptNow()[1]).toEqual(answer);
    expect(aiProviders).toHaveBeenCalled();
  });

  it("shows the activity line from polled events too (ai D30)", async () => {
    let finish: (v: unknown) => void = () => undefined;
    aiTurnPoll
      .mockResolvedValueOnce({ events: [{ event: "tool", data: { id: "t1", name: "search_notes", input: { text: "roof" } } }], next: 1, done: false })
      .mockImplementationOnce(() => new Promise((r) => (finish = r)));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Hi" } });
    fireEvent.click(m.getByRole("button", { name: "Send" }));
    const status = () => m.container.querySelector(".ai-activity [role=status]");
    await waitFor(() => expect(status()).toHaveTextContent("Searching notes…"));
    finish({ events: [{ event: "done", data: { stop: "end_turn", messages: [] } }], next: 2, done: true });
    await waitFor(() => expect(m.getByRole("button", { name: "Send" })).toBeInTheDocument());
    expect(status()).toBeNull();
  });

  it("stops the turn on the server", async () => {
    aiTurnPoll.mockImplementation(
      (_turn: string, _after: number, signal?: AbortSignal) =>
        new Promise((_, reject) => signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")))),
    );
    const m = render(() => <ChatPanel noteId={() => ""} />);
    fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Hi" } });
    fireEvent.click(m.getByRole("button", { name: "Send" }));
    fireEvent.click(await m.findByRole("button", { name: "Stop" }));
    await waitFor(() => expect(m.getByText("Stopped.")).toBeInTheDocument());
    await waitFor(() => expect(aiTurnStop).toHaveBeenCalledWith("turn-1"));
  });

  it("says the chat needs the home server when it cannot be reached", async () => {
    const { ApiError } = await import("../api");
    const message = "The AI chat needs the home server, which is not reachable.";
    aiTurnStart.mockRejectedValue(new ApiError(503, { error: "ai_needs_server", message }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Hi" } });
    fireEvent.click(m.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(m.getByRole("alert")).toHaveTextContent(message));
  });

  it("shows Set up AI in the web app, linking to the home server, when nothing is configured", async () => {
    aiProviders.mockResolvedValue({
      ...STATUS,
      default_provider: null,
      providers: { anthropic: { configured: false, model: null }, openai: { configured: false, model: null }, chatgpt: { configured: false, model: null } },
    });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    const link = await m.findByRole("link", { name: "Set up AI in the web app" });
    expect(link).toHaveAttribute("href", "https://home.example/settings/ai");
    expect(m.queryByLabelText("Model")).toBeNull();
  });

  it("asks for a sync round after Apply, and refreshes open notes when it ends", async () => {
    aiApply.mockResolvedValue({ op: "update", note_id: "01JNOTE", version: "v2" });
    const asked = Date.now();
    appSyncNow.mockResolvedValue({ state: "syncing", phase: "syncing", last_attempt_ms: asked - 1000 });
    appSync
      .mockResolvedValueOnce({ state: "syncing", phase: "syncing", last_attempt_ms: asked - 1000 })
      .mockResolvedValue({ state: "synced", phase: "synced", last_attempt_ms: asked + 60_000 });
    polls([{ event: "proposal", data: PROPOSAL }]);
    const elsewhere = vi.fn();
    const off = onNoteChangedElsewhere(elsewhere);
    const m = render(() => <ChatPanel noteId={() => ""} />);
    fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Rename it." } });
    fireEvent.click(m.getByRole("button", { name: "Send" }));
    const card = await m.findByRole("region", { name: "Proposed edit to Groceries" });
    fireEvent.click(card.querySelector("button.primary")!);
    await waitFor(() => expect(m.getByText("Applied.")).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith({ ...PROPOSAL, op: "update" });
    expect(updateNote).not.toHaveBeenCalled();
    await waitFor(() => expect(appSyncNow).toHaveBeenCalled());
    await waitFor(() => expect(elsewhere).toHaveBeenCalledWith(null), { timeout: 3000 });
    expect(appSync.mock.calls.length).toBeGreaterThanOrEqual(2);
    off();
  });

  it("asks for a sync round after a change the AI made itself", async () => {
    appSyncNow.mockResolvedValue({ state: "synced", phase: "synced", last_attempt_ms: Date.now() + 60_000 });
    appSync.mockResolvedValue({ state: "synced", phase: "synced", last_attempt_ms: Date.now() + 60_000 });
    polls([{ event: "applied", data: { id: "toolu_x", op: "update", note_id: "01JNOTE", title: "Groceries", version: "v2" } }]);
    const elsewhere = vi.fn();
    const off = onNoteChangedElsewhere(elsewhere);
    const m = render(() => <ChatPanel noteId={() => ""} />);
    fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Rename it." } });
    fireEvent.click(m.getByRole("button", { name: "Send" }));
    await m.findByRole("region", { name: "Edited Groceries" });
    await waitFor(() => expect(appSyncNow).toHaveBeenCalled());
    await waitFor(() => expect(elsewhere).toHaveBeenCalledWith(null));
    off();
  });
});

describe("markdown", () => {
  it("reads blocks and inline marks", () => {
    expect(parseBlocks("# Title\n\nOne\ntwo\n\n- a\n- b\n\n```\ncode\n```")).toEqual([
      { kind: "h", level: 1, text: "Title" },
      { kind: "p", text: "One two" },
      { kind: "ul", items: ["a", "b"] },
      { kind: "code", text: "code" },
    ]);
    expect(parseInline("see [Roof](/notes/01J) and `x`")).toEqual([
      { kind: "text", text: "see " },
      { kind: "link", href: "/notes/01J", children: [{ kind: "text", text: "Roof" }] },
      { kind: "text", text: " and " },
      { kind: "code", text: "x" },
    ]);
  });
});

describe("diff", () => {
  it("lines up a content block's text", () => {
    const before = { kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text: "Milk" } }] };
    const after = {
      kind: "content",
      labels: ["body"],
      blocks: [
        { kind: "paragraph", fields: { text: "Milk" } },
        { kind: "paragraph", fields: { text: "Eggs" } },
      ],
    };
    expect(lineDiff(blockLines(before), blockLines(after))).toEqual([
      { type: "same", text: "Milk" },
      { type: "add", text: "Eggs" },
    ]);
  });
});
