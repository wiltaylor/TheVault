import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";
import { turnsOver } from "./fakeTurns";

// The chat panel's web steps and downloads (ai D27): a fetch_url step reads "Read <host>" with
// the address linking out; a download_asset proposal is a card whose Apply sends it to
// POST /api/ai/apply, which stores the file as an asset on the home server; a download the AI
// made itself shows the new asset, with an image's thumbnail.

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

const turns = turnsOver(streamEvents);

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, streamEvents, streamTurn: turns.streamTurn, api: { ...real.api, aiTurnStart: turns.aiTurnStart, aiTurnStop: turns.aiTurnStop, aiApply, aiModels, aiProviders, app } };
});

const { ChatPanel, stepLabel } = await import("./ChatPanel");
const { newConversation } = await import("./store");
const { fileSize } = await import("./DownloadCard");

const PROPOSAL = {
  id: "toolu_d",
  op: "download",
  url: "https://example.com/files/sunset.png",
  filename: "sunset.png",
  media_type: "image/png",
  size: 2048,
};

async function ask(m: ReturnType<typeof render>) {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Get it." } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
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

describe("web steps", () => {
  it("labels the read and download steps by host", () => {
    expect(stepLabel("fetch_url", { url: "https://example.com/a/b?c=1" })).toBe("Read example.com");
    expect(stepLabel("download_asset", { url: "https://files.example.org/x.pdf", filename: null })).toBe("Download from files.example.org");
    expect(stepLabel("fetch_url", { url: "not a url" })).toBe("Read not a url");
  });

  it("shows Read <host> with the URL linking out, and the page's title", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    live!.onEvent({ event: "tool", data: { id: "toolu_f", name: "fetch_url", input: { url: "https://example.com/roofs" } } });
    const link = await m.findByRole("link", { name: "example.com" });
    expect(link).toHaveAttribute("href", "https://example.com/roofs");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
    expect(link.closest("summary")).toHaveTextContent("Read example.com (running)");
    live!.onEvent({
      event: "tool_result",
      data: {
        id: "toolu_f",
        is_error: false,
        summary: { notes: [], web: { url: "https://example.com/roofs/", host: "example.com", media_type: "text/html", kind: "html", title: "Slate roofs" } },
      },
    });
    await waitFor(() => expect(link.closest("summary")).toHaveTextContent("Read example.com (Slate roofs)"));
    expect(m.getByRole("link", { name: "https://example.com/roofs/" })).toHaveAttribute("href", "https://example.com/roofs/");
  });

  it("does not link a URL that is not http or https", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    live!.onEvent({ event: "tool", data: { id: "toolu_f", name: "fetch_url", input: { url: "javascript:alert(1)" } } });
    await m.findByText(/Read javascript:alert\(1\)/);
    expect(m.queryByRole("link", { name: /javascript/ })).toBeNull();
  });

  it("shows a refused fetch as a failed step with the reason", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    live!.onEvent({ event: "tool", data: { id: "toolu_f", name: "fetch_url", input: { url: "http://192.168.1.1/" } } });
    live!.onEvent({
      event: "tool_result",
      data: { id: "toolu_f", is_error: true, summary: { notes: [], message: "TheVault does not fetch addresses on the local network.", web: { url: "http://192.168.1.1/" } } },
    });
    expect(await m.findByText("TheVault does not fetch addresses on the local network.")).toBeTruthy();
    expect(m.getByText("(failed)")).toBeTruthy();
  });
});

describe("download cards", () => {
  it("shows the proposal and applies it through POST /api/ai/apply", async () => {
    aiApply.mockResolvedValue({ op: "download", asset_id: "01JASSET", filename: "sunset.png", media_type: "image/png", size: 2048 });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    live!.onEvent({ event: "tool", data: { id: "toolu_d", name: "download_asset", input: { url: PROPOSAL.url, filename: null } } });
    live!.onEvent({ event: "proposal", data: PROPOSAL });
    const card = await m.findByRole("region", { name: "Proposed download of sunset.png" });
    expect(card).toHaveTextContent("image/png, 2.0 KB");
    expect(m.getByRole("link", { name: PROPOSAL.url })).toHaveAttribute("href", PROPOSAL.url);
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(aiApply).toHaveBeenCalledWith(PROPOSAL));
    const done = await m.findByRole("region", { name: "Downloaded sunset.png" });
    expect(done).toHaveTextContent("Stored in the vault as an asset.");
    for (const l of m.getAllByRole("link", { name: "sunset.png" })) expect(l).toHaveAttribute("href", expect.stringContaining("01JASSET"));
    expect(m.getByRole("img", { name: "sunset.png" })).toHaveAttribute("src", expect.stringContaining("01JASSET"));
    expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
  });

  it("shows a failed apply and keeps Apply for a retry, and Discard ends it", async () => {
    aiApply.mockRejectedValue(new Error("the address could not be fetched: connection closed before message completed"));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    live!.onEvent({ event: "proposal", data: { ...PROPOSAL, media_type: null, size: null } });
    const card = await m.findByRole("region", { name: "Proposed download of sunset.png" });
    expect(card).toHaveTextContent("size unknown");
    fireEvent.click(m.getByRole("button", { name: "Apply" }));
    expect((await m.findByRole("alert")).textContent).toContain("connection closed");
    fireEvent.click(m.getByRole("button", { name: "Discard" }));
    expect(await m.findByText("Discarded.")).toBeTruthy();
    expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
  });

  it("applies a download with the other proposals under Apply all", async () => {
    aiApply.mockResolvedValue({ op: "download", asset_id: "01JASSET", filename: "sunset.png" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    live!.onEvent({ event: "proposal", data: PROPOSAL });
    live!.onEvent({ event: "proposal", data: { ...PROPOSAL, id: "toolu_e", filename: "sunrise.png", url: "https://example.com/sunrise.png" } });
    fireEvent.click(await m.findByRole("button", { name: "Apply all (2)" }));
    await waitFor(() => expect(aiApply).toHaveBeenCalledTimes(2));
  });

  it("shows a download the AI made itself, without Apply", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    live!.onEvent({
      event: "applied",
      data: { id: "toolu_d", op: "download", asset_id: "01JAUTO", filename: "report.pdf", media_type: "application/pdf", size: 3 * 1024 * 1024, url: "https://example.com/report.pdf" },
    });
    const card = await m.findByRole("region", { name: "Downloaded report.pdf" });
    expect(card).toHaveTextContent("Downloaded without asking, as Settings > AI allows.");
    expect(card).toHaveTextContent("application/pdf, 3.0 MB");
    expect(m.getByRole("link", { name: "report.pdf" })).toHaveAttribute("href", expect.stringContaining("01JAUTO"));
    expect(m.queryByRole("img")).toBeNull();
    expect(m.queryByRole("button", { name: "Apply" })).toBeNull();
  });

  it("formats sizes", () => {
    expect(fileSize(10)).toBe("10 bytes");
    expect(fileSize(1536)).toBe("1.5 KB");
    expect(fileSize(null)).toBe("size unknown");
  });
});
