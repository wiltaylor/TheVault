import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";

// The activity line above the chat box (ai D30): while a turn runs, a spinner, the current
// step's verb from the last event and the seconds since the turn started; gone when the turn
// ends with done, error, a refusal or Stop. Only the verb is in the status region.

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
const aiProviders = vi.fn();
const app = vi.fn(async () => null);

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, streamEvents, api: { ...real.api, aiModels, aiProviders, app } };
});

const { ChatPanel, activeLabel } = await import("./ChatPanel");
const { newConversation } = await import("./store");

async function ask(m: ReturnType<typeof render>) {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "What did I decide?" } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
}

/** The activity line's status region: the verb alone. */
const verb = (m: ReturnType<typeof render>) => m.queryByText(/…$/, { selector: ".ai-activity [role=status]" });

beforeEach(() => {
  aiProviders.mockResolvedValue({ default_provider: "anthropic", providers: { anthropic: { configured: true, model: "m" } } });
  aiModels.mockResolvedValue({ provider: "anthropic", model: "m", models: [{ id: "m", display_name: "M" }] });
});

afterEach(() => {
  vi.useRealTimers();
  newConversation();
  live = null;
  cleanup();
  vi.clearAllMocks();
});

describe("activity line", () => {
  it("names the step from the last event while the turn runs, and goes at done", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    expect(verb(m)).toBeNull();
    await ask(m);
    await waitFor(() => expect(verb(m)).toHaveTextContent("Thinking…"));
    expect(verb(m)).toHaveAttribute("aria-live", "polite");
    expect(verb(m)).not.toHaveTextContent(/\ds/);
    live!.onEvent({ event: "tool", data: { id: "t1", name: "search_notes", input: { text: "roof" } } });
    await waitFor(() => expect(verb(m)).toHaveTextContent("Searching notes…"));
    live!.onEvent({ event: "tool_result", data: { id: "t1", is_error: false, summary: { notes: [] } } });
    await waitFor(() => expect(verb(m)).toHaveTextContent("Thinking…"));
    live!.onEvent({ event: "tool", data: { id: "t2", name: "fetch_url", input: { url: "https://example.com" } } });
    await waitFor(() => expect(verb(m)).toHaveTextContent("Reading a web page…"));
    live!.onEvent({ event: "tool_result", data: { id: "t2", is_error: false, summary: { notes: [] } } });
    live!.onEvent({ event: "text", data: { delta: "Slate." } });
    await waitFor(() => expect(verb(m)).toHaveTextContent("Writing…"));
    // Stop stays in the form, under the line.
    expect(m.getByRole("button", { name: "Stop" })).toBeInTheDocument();
    live!.onEvent({ event: "done", data: { messages: [] } });
    await waitFor(() => expect(verb(m)).toBeNull());
    live!.end();
    await waitFor(() => expect(m.getByRole("button", { name: "Send" })).toBeInTheDocument());
    expect(m.container.querySelector(".ai-activity")).toBeNull();
  });

  it("counts the seconds since the turn started", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    const line = await waitFor(() => m.container.querySelector(".ai-activity")!);
    expect(line).toHaveTextContent("Thinking… 0s");
    vi.advanceTimersByTime(4000);
    await waitFor(() => expect(line).toHaveTextContent("Thinking… 4s"));
    live!.onEvent({ event: "tool", data: { id: "t1", name: "search_notes", input: { text: "roof" } } });
    vi.advanceTimersByTime(3000);
    await waitFor(() => expect(line).toHaveTextContent("Searching notes… 7s"));
  });

  it("goes when the turn ends in an error", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    await waitFor(() => expect(verb(m)).not.toBeNull());
    live!.onEvent({ event: "error", data: { kind: "rate_limit", message: "Slow down.", messages: [] } });
    await waitFor(() => expect(verb(m)).toBeNull());
  });

  it("goes when the turn ends in a refusal", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    await waitFor(() => expect(verb(m)).not.toBeNull());
    live!.onEvent({ event: "refusal", data: { provider: "anthropic", model: "m", category: "cyber", explanation: null, messages: [] } });
    await waitFor(() => expect(verb(m)).toBeNull());
  });

  it("goes at Stop", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    await waitFor(() => expect(verb(m)).not.toBeNull());
    fireEvent.click(m.getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(m.getByRole("button", { name: "Send" })).toBeInTheDocument());
    expect(verb(m)).toBeNull();
    expect(m.container.querySelector(".ai-activity")).toBeNull();
  });

  it("words each tool's step in the present", () => {
    expect(activeLabel("search_notes", {})).toBe("Searching notes");
    expect(activeLabel("propose_archive", { archived: false })).toBe("Checking an unarchive");
    expect(activeLabel("web_search", { query: "x" })).toBe("Searching the web");
    expect(activeLabel("mystery", {})).toBe("Running mystery");
  });
});
