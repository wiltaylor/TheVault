import { cleanup, fireEvent, render, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";

// What the user applied reaches the model (ai D29): Apply, Apply all and Discard record each
// proposal's outcome, and the next message starts with one text block of `[Applied by the
// user: ...]` lines ahead of the question, shown collapsed as "Applied changes" and sent once.

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

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, streamEvents, api: { ...real.api, aiApply, aiModels, aiProviders, app } };
});

const { ChatPanel } = await import("./ChatPanel");
const { newConversation, outcomesNow } = await import("./store");

const DOWNLOAD = {
  id: "toolu_d",
  op: "download",
  url: "https://example.com/elden-ring-cover.jpg",
  filename: "elden-ring-cover.jpg",
  media_type: "image/jpeg",
  size: 2048,
};

const DOC = { fields: { title: "Groceries" }, blocks: [] };
const EDIT = {
  id: "toolu_e",
  note_id: "01JNOTE",
  title: "Groceries",
  base_version: "v2",
  before: DOC,
  after: { fields: { title: "Groceries list" }, blocks: [] },
  ops: [{ op: "field", name: "title", before: "Groceries", after: "Groceries list" }],
};

type Body = { messages: { role: string; content: { type: string; text?: string }[] }[] };

/** Send `text` and wait for its turn to start: the body it sent. */
async function send(m: ReturnType<typeof render>, text: string): Promise<Body> {
  live = null;
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: text } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
  return streamEvents.mock.calls.at(-1)![1] as Body;
}

/** End the turn in flight with an answer. */
async function finish(m: ReturnType<typeof render>) {
  live!.onEvent({ event: "done", data: { messages: [{ role: "assistant", content: [{ type: "text", text: "Done." }] }] } });
  live!.end();
  await waitFor(() => expect(m.getByRole("button", { name: "Send" })).toBeInTheDocument());
}

const lastUser = (b: Body) => b.messages.filter((x) => x.role === "user").at(-1)!;

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

describe("applied changes", () => {
  it("sends an applied download and a discarded edit ahead of the next message, once", async () => {
    aiApply.mockResolvedValue({ op: "download", asset_id: "01JASSET", filename: "elden-ring-cover.jpg", media_type: "image/jpeg", size: 2048 });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    const first = await send(m, "Get the cover and rename the list.");
    expect(lastUser(first).content).toEqual([{ type: "text", text: "Get the cover and rename the list." }]);
    live!.onEvent({ event: "proposal", data: DOWNLOAD });
    live!.onEvent({ event: "proposal", data: EDIT });
    await finish(m);

    const download = await m.findByRole("region", { name: "Proposed download of elden-ring-cover.jpg" });
    fireEvent.click(within(download).getByRole("button", { name: "Apply" }));
    await m.findByRole("region", { name: "Downloaded elden-ring-cover.jpg" });
    const edit = m.getByRole("region", { name: "Proposed edit to Groceries" });
    fireEvent.click(within(edit).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(outcomesNow()).toHaveLength(2));

    const second = await send(m, "Attach it to the note.");
    const lines = [
      "[Applied by the user: download of https://example.com/elden-ring-cover.jpg stored as asset 01JASSET (elden-ring-cover.jpg)]",
      "[Discarded by the user: edit of note 01JNOTE]",
    ];
    expect(lastUser(second).content).toEqual([
      { type: "text", text: lines.join("\n") },
      { type: "text", text: "Attach it to the note." },
    ]);
    // The panel shows the lines collapsed, apart from the user's words.
    const shown = m.getByText("Applied changes").closest("details")!;
    expect(shown).not.toHaveAttribute("open");
    expect(shown).toHaveTextContent(lines[0]!);
    expect(m.getByText("Attach it to the note.")).not.toHaveTextContent("Applied by the user");
    await finish(m);

    // Sent once: the message after carries the question alone, and the history keeps the lines.
    const third = await send(m, "Thanks.");
    expect(lastUser(third).content).toEqual([{ type: "text", text: "Thanks." }]);
    expect(third.messages.filter((x) => x.role === "user")[1]!.content[0]!.text).toBe(lines.join("\n"));
    expect(m.getAllByText("Applied changes")).toHaveLength(1);
  });

  it("records an applied edit's new version, a created note's id and a failed apply", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await send(m, "Do both.");
    live!.onEvent({ event: "proposal", data: EDIT });
    live!.onEvent({ event: "proposal", data: { id: "toolu_c", op: "create", type: "game", title: "Hades II", after: { fields: { title: "Hades II" }, blocks: [] } } });
    live!.onEvent({ event: "proposal", data: { ...EDIT, id: "toolu_f", note_id: "01JOTHER", title: "Other" } });
    await finish(m);
    aiApply
      .mockResolvedValueOnce({ op: "update", note_id: "01JNOTE", version: "v3" })
      .mockResolvedValueOnce({ op: "create", note_id: "01JHADES", version: "v1" })
      .mockRejectedValueOnce(new Error("the server is down"));
    fireEvent.click(m.getByRole("button", { name: "Apply all (3)" }));
    await waitFor(() => expect(outcomesNow()).toHaveLength(3));
    expect(outcomesNow()).toEqual([
      "[Applied by the user: edit of note 01JNOTE now at version v3]",
      '[Applied by the user: created note 01JHADES "Hades II", version v1]',
      "[Failed to apply: edit of note 01JOTHER: the server is down]",
    ]);
  });

  it("forgets the outcomes with the conversation", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await send(m, "Get it.");
    live!.onEvent({ event: "proposal", data: DOWNLOAD });
    await finish(m);
    fireEvent.click(m.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(outcomesNow()).toHaveLength(1));
    newConversation();
    expect(outcomesNow()).toEqual([]);
  });

  it("shows a find_assets step with the files it found, linking to each", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await send(m, "Find the cover.");
    live!.onEvent({ event: "tool", data: { id: "toolu_a", name: "find_assets", input: { text: "elden", limit: 20 } } });
    live!.onEvent({
      event: "tool_result",
      data: { id: "toolu_a", is_error: false, summary: { notes: [], total: 2, assets: [{ id: "01JA", filename: "elden-a.jpg" }, { id: "01JB", filename: "elden-b.jpg" }] } },
    });
    const step = await m.findByText("(2 found)");
    expect(step.closest("summary")).toHaveTextContent("Looked for files named “elden”");
    expect(m.getByRole("link", { name: "elden-b.jpg" })).toHaveAttribute("href", expect.stringContaining("01JB"));
  });
});
