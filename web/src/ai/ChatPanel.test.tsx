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
const aiModels = vi.fn();
const aiStatus = vi.fn();

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, streamEvents, api: { ...real.api, updateNote, aiModels, aiStatus } };
});

const { ChatPanel } = await import("./ChatPanel");
const { newConversation, transcriptNow } = await import("./store");
const { parseBlocks, parseInline } = await import("./markdown");
const { blockLines, lineDiff } = await import("./diff");

const STATUS = {
  secrets_key: true,
  default_provider: "anthropic",
  providers: {
    anthropic: { configured: true, model: "claude-opus-5" },
    openai: { configured: false, model: null },
    chatgpt: { configured: false, model: null, account: null },
  },
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
  aiStatus.mockResolvedValue(STATUS);
  aiModels.mockResolvedValue(MODELS);
});

afterEach(() => {
  newConversation();
  live = null;
  cleanup();
  vi.clearAllMocks();
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

  it("applies a proposal through the notes write API with its base version", async () => {
    updateNote.mockResolvedValue({ id: "01JNOTE", version: "v2" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m);
    emit("proposal", PROPOSAL);
    const card = await m.findByRole("region", { name: "Proposed edit to Groceries" });
    expect(card).toHaveTextContent("Groceries and more");
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(m.getByText("Applied.")).toBeInTheDocument());
    expect(updateNote).toHaveBeenCalledWith("01JNOTE", PROPOSAL.after, "v1");
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
