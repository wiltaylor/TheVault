import { cleanup, fireEvent, render, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";
import { turnsOver } from "./fakeTurns";

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
const aiAccounts = vi.fn();
const app = vi.fn();
const turns = turnsOver(streamEvents);
const { aiTurnStart, aiTurnStop, streamTurn } = turns;
const aiTurnPoll = vi.fn();
const appSync = vi.fn();
const appSyncNow = vi.fn();

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    streamEvents,
    streamTurn,
    api: { ...real.api, updateNote, aiApply, aiModels, aiAccounts, app, aiTurnStart, aiTurnPoll, aiTurnStop, appSync, appSyncNow },
  };
});

const { ChatPanel } = await import("./ChatPanel");
const { newConversation, transcriptNow, forgetApp } = await import("./store");
const { forgetAppInfo } = await import("../inApp");
const { generalActions } = await import("../palette/actions");
const { changes, onNoteChangedElsewhere } = await import("../changes");
const { renderMarkdown } = await import("./markdown");
const { blockLines, lineDiff } = await import("./diff");

const ANTHROPIC = { id: "a1", name: "Anthropic", kind: "anthropic", kind_label: "Anthropic API", model: "claude-opus-5", configured: true };
const STATUS = {
  default: "a1",
  accounts: [
    ANTHROPIC,
    { id: "o1", name: "OpenAI", kind: "openai", kind_label: "OpenAI API", model: null, configured: false },
    {
      id: "c1",
      name: "ChatGPT",
      kind: "chatgpt",
      kind_label: "ChatGPT subscription",
      model: null,
      configured: true,
      usage: {
        read_at_ms: 0,
        plan: "plus",
        windows: [
          { label: "5 h", used_percent: 12, window_minutes: 300, resets_at_ms: null },
          { label: "Weekly", used_percent: 41.6, window_minutes: 10080, resets_at_ms: null },
        ],
      },
    },
  ],
  confirm: { notes: true, archive: true, delete: true, types: true, downloads: true, taxonomies: true, agents: true, skills: true },
};

const MODELS = {
  account: "a1",
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
  aiAccounts.mockResolvedValue(STATUS);
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
    expect(live!.body).toMatchObject({ account: "a1", model: "claude-opus-5", note_id: "01JOPEN", messages: [{ role: "user" }] });
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

  it("shows a draft step as a compact card linking to the Outbox item", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m);
    emit("tool", { id: "d1", name: "draft_email", input: { to: ["01JANN"] } });
    emit("tool_result", {
      id: "d1",
      is_error: false,
      summary: {
        notes: [{ id: "01JITEM", title: "Lunch" }],
        draft: { note_id: "01JITEM", kind: "email", status: "draft", title: "Lunch", to: ["Ann Lee"] },
      },
    });
    const card = await m.findByRole("group", { name: "Outbox draft" });
    expect(card).toHaveTextContent("Drafted an email to Ann Lee");
    expect(card).not.toHaveTextContent("for review");
    expect(within(card).getByRole("link", { name: "Open it in the Outbox" })).toHaveAttribute("href", "/outbox?item=01JITEM");
    // An agent that sends without review says so; a failed draft stays a step.
    emit("tool", { id: "d2", name: "draft_sms", input: {} });
    emit("tool_result", {
      id: "d2",
      is_error: false,
      summary: { notes: [], draft: { note_id: "01JSMS", kind: "sms", status: "ready", title: "SMS to Bob", to: ["Bob"] } },
    });
    await waitFor(() => expect(m.getAllByRole("group", { name: "Outbox draft" })).toHaveLength(2));
    expect(m.getAllByRole("group", { name: "Outbox draft" })[1]).toHaveTextContent("Drafted an SMS to Bob, sending without review");
    emit("tool", { id: "d3", name: "draft_event", input: {} });
    emit("tool_result", { id: "d3", is_error: true, summary: { notes: [], message: "a new event needs a title" } });
    await waitFor(() => expect(m.getByText("Drafted a calendar invite")).toBeInTheDocument());
    expect(m.getAllByRole("group", { name: "Outbox draft" })).toHaveLength(2);
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

  it("shows a browser_run step as its command, with its output in the step's collapsible body", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m);
    emit("tool", { id: "b1", name: "browser_run", input: { command: "playwright-cli snapshot", timeout: null } });
    expect(await m.findByText(/Running a command in the browser box/)).toBeInTheDocument();
    emit("tool_result", {
      id: "b1",
      is_error: false,
      summary: {
        notes: [],
        browser: { kind: "run", command: "playwright-cli snapshot", exit: 0, ms: 812, timed_out: false, stdout: "- button \"Go\" [ref=e21]", stderr: "slow page" },
      },
    });
    await waitFor(() => expect(m.getByText("Ran playwright-cli snapshot in the browser box")).toBeInTheDocument());
    expect(m.getByText("(exit 0)")).toBeInTheDocument();
    const step = m.getByText("Ran playwright-cli snapshot in the browser box").closest("details")!;
    expect(step.open).toBe(false);
    expect(within(step).getByLabelText("Output")).toHaveTextContent('- button "Go" [ref=e21]');
    expect(within(step).getByLabelText("Errors")).toHaveTextContent("slow page");
    // The step links to the browser box's desktop in the app, to watch the agent beside the chat.
    const watch = within(step).getByRole("link", { name: "Watch" });
    expect(watch).toHaveAttribute("href", "/browser-box");
    // A command that timed out says so.
    emit("tool", { id: "b2", name: "browser_run", input: { command: "sleep 999\necho done", timeout: 1 } });
    emit("tool_result", {
      id: "b2",
      is_error: false,
      summary: { notes: [], browser: { kind: "run", command: "sleep 999", exit: 137, ms: 1000, timed_out: true, stdout: "", stderr: "" } },
    });
    await waitFor(() => expect(m.getByText("(timed out)")).toBeInTheDocument());
    expect(m.getByText("Ran sleep 999… in the browser box")).toBeInTheDocument();
  });

  it("shows a browser_file image as the thumbnail its step carries, and nothing else as an image", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m);
    const thumb = "data:image/jpeg;base64,/9j/AAAA";
    emit("tool", { id: "f1", name: "browser_file", input: { path: "shot.png" } });
    emit("tool_result", {
      id: "f1",
      is_error: false,
      summary: { notes: [], browser: { kind: "image", path: "shot.png", media_type: "image/png", size: 900, thumbnail: thumb } },
    });
    const img = await m.findByRole("img", { name: "shot.png" });
    expect(img).toHaveAttribute("src", thumb);
    expect(m.getByText("Read shot.png from the browser box")).toBeInTheDocument();
    expect(m.getByText("(image/png)")).toBeInTheDocument();
    // A thumbnail that is not the server's JPEG data URL is not shown.
    emit("tool", { id: "f2", name: "browser_file", input: { path: "x.png" } });
    emit("tool_result", {
      id: "f2",
      is_error: false,
      summary: { notes: [], browser: { kind: "image", path: "x.png", media_type: "image/png", size: 9, thumbnail: "https://evil.example/x.png" } },
    });
    await waitFor(() => expect(m.getByText("Read x.png from the browser box")).toBeInTheDocument());
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
      account: "a1",
      name: "Anthropic",
      kind: "anthropic",
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

  it("sends /update-skills with the transcript and no message, keeping the server's message for the next turn", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await askIt(m, "/update-skills");
    expect(live!.body).toMatchObject({ update_skills: true, messages: [] });
    const convo = m.getByRole("list", { name: "Conversation" });
    await waitFor(() => expect(convo).toHaveTextContent("Updating skills…"));
    const message = {
      role: "user",
      content: [
        { type: "notice", kind: "skills_updated", text: "Skills updated (3)" },
        { type: "text", text: "The available skills changed; the list at the end of your instructions is current." },
      ],
    };
    emit("skills_updated", { count: 3, skills: ["a", "b", "c"], message });
    await waitFor(() => expect(within(convo).getByText("Skills updated (3)")).toHaveClass("ai-skills-updated"));
    expect(convo).not.toHaveTextContent("/update-skills");
    expect(convo.querySelector(".ai-question")).toBeNull();
    const answer = { role: "assistant", content: [{ type: "text", text: "Noted." }] };
    emit("done", { stop: "end_turn", messages: [answer] });
    live!.end();
    await waitFor(() => expect(m.getByRole("button", { name: "Send" })).toBeInTheDocument());
    expect(transcriptNow()).toEqual([message, answer]);
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

  it("offers the accounts set up, the default account's default model, and how much of a plan is used", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await waitFor(() => expect(m.getByLabelText("Model")).toHaveValue("claude-opus-5"));
    expect(m.getByLabelText("Account")).toHaveValue("a1");
    expect((m.getByRole("option", { name: "OpenAI" }) as HTMLOptionElement).disabled).toBe(true);
    expect(m.getByRole("option", { name: "ChatGPT (42% used)" })).toBeInTheDocument();
    expect(m.queryByTestId("account-used")).toBeNull();
    expect(m.getByText(/Answers come from Anthropic;/)).toBeInTheDocument();

    // A ChatGPT account with a reading shows its most used limit beside the picker, and the turn names it.
    aiModels.mockResolvedValue({ account: "c1", model: "gpt-6", models: [{ id: "gpt-6", display_name: null }] });
    fireEvent.change(m.getByLabelText("Account"), { target: { value: "c1" } });
    await waitFor(() => expect(m.getByLabelText("Model")).toHaveValue("gpt-6"));
    expect(aiModels).toHaveBeenLastCalledWith("c1");
    expect(m.getByTestId("account-used")).toHaveTextContent("42% used");
    await askIt(m);
    expect(live!.body).toMatchObject({ account: "c1", model: "gpt-6" });
    emit("done", { stop: "end_turn", messages: [] });
    live!.end();
    // The panel keeps its choice: back to the default account for the tests that follow.
    aiModels.mockResolvedValue(MODELS);
    await waitFor(() => expect(m.getByRole("button", { name: "Send" })).toBeInTheDocument());
    fireEvent.change(m.getByLabelText("Account"), { target: { value: "a1" } });
    await waitFor(() => expect(m.getByLabelText("Model")).toHaveValue("claude-opus-5"));
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
    aiTurnStart.mockResolvedValue({ turn: "turn-1" });
  });
  // Back to the fake turns the other tests run.
  afterEach(() => aiTurnStart.mockReset());

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
    expect(streamTurn).not.toHaveBeenCalled();
    expect(aiTurnStart).toHaveBeenCalledWith(expect.objectContaining({ account: "a1", note_id: "01JOPEN", messages: [expect.objectContaining({ role: "user" })] }));
    expect(aiTurnPoll.mock.calls.map((c) => c.slice(0, 2))).toEqual([
      ["turn-1", 0],
      ["turn-1", 2],
    ]);
    expect(transcriptNow()[1]).toEqual(answer);
    expect(aiAccounts).toHaveBeenCalled();
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
    aiAccounts.mockResolvedValue({ ...STATUS, default: null, accounts: [] });
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
  const dom = (text: string) => {
    const div = document.createElement("div");
    div.innerHTML = renderMarkdown(text);
    return div;
  };

  it("renders headings, paragraphs, nested and task lists, quotes, rules and code", () => {
    const d = dom("# Title\n\nOne **two** ~~three~~\n\n- a\n  - b\n- [x] done\n\n> quoted *text*\n\n---\n\n```\ncode\n```");
    expect(d.querySelector("h1")?.textContent).toBe("Title");
    expect(d.querySelector("strong")?.textContent).toBe("two");
    expect(d.querySelector("del")?.textContent).toBe("three");
    expect(d.querySelector("ul ul li")?.textContent).toBe("b");
    expect((d.querySelector("input[type=checkbox]") as HTMLInputElement).disabled).toBe(true);
    expect(d.querySelector("blockquote em")?.textContent).toBe("text");
    expect(d.querySelector("hr")).not.toBeNull();
    expect(d.querySelector("pre code")?.textContent).toBe("code\n");
  });

  it("renders a table in a scrolling box", () => {
    const d = dom("| Game | Year |\n|---|---:|\n| Hades II | 2025 |\n| Elden Ring | 2022 |");
    const table = d.querySelector(".ai-md-table > table")!;
    expect([...table.querySelectorAll("th")].map((th) => th.textContent)).toEqual(["Game", "Year"]);
    expect([...table.querySelectorAll("tbody tr")].map((tr) => [...tr.querySelectorAll("td")].map((td) => td.textContent))).toEqual([
      ["Hades II", "2025"],
      ["Elden Ring", "2022"],
    ]);
  });

  it("keeps app and web links, drops unsafe ones, and opens other sites in a new tab", () => {
    const d = dom("[Roof](/notes/01J) [site](https://example.org) [bad](javascript:alert(1)) <script>alert(1)</script><b onclick=\"x()\">b</b>");
    const links = [...d.querySelectorAll("a")];
    expect(links.map((a) => a.getAttribute("href"))).toEqual(["/notes/01J", "https://example.org"]);
    expect(links[0]!.getAttribute("target")).toBeNull();
    expect(links[1]!.getAttribute("target")).toBe("_blank");
    expect(links[1]!.getAttribute("rel")).toContain("noopener");
    expect(d.textContent).toContain("bad");
    expect(d.querySelector("script")).toBeNull();
    expect(d.innerHTML).not.toContain("onclick");
  });

  it("shows the vault's images and only the alt text of another site's", () => {
    const d = dom("![cover](/api/assets/01ABC) ![remote](https://tracker.example/pixel.png)");
    const imgs = [...d.querySelectorAll("img")];
    expect(imgs.map((i) => i.getAttribute("src"))).toEqual(["/api/assets/01ABC"]);
    expect(d.textContent).toContain("remote");
  });

  it("renders an unfinished answer as it streams", () => {
    const d = dom("Here:\n\n```\nhalf");
    expect(d.querySelector("pre code")?.textContent).toContain("half");
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
