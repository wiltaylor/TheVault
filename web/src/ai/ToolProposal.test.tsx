import { cleanup, fireEvent, render, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";
import { turnsOver } from "./fakeTurns";

// The chat panel's script tool proposal card (script-tools T3): the tool file's diff and the
// script's, a delete with whether its script goes; Apply sends it to POST /api/ai/apply, a tool
// changed since the proposal is not applied, Discard writes nothing, the outcome reaches the next
// message, a change the AI made itself links to the tool, and a script step shows how it ran.

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
const putTool = vi.fn();
const skills = vi.fn(async () => []);
const agents = vi.fn(async () => ({ agents: [], tools: [] }));
const aiModels = vi.fn();
const aiAccounts = vi.fn();
const app = vi.fn(async () => null);
const reloadDefinitions = vi.fn(async () => undefined);

const turns = turnsOver(streamEvents);

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    streamEvents,
    streamTurn: turns.streamTurn,
    api: { ...real.api, aiTurnStart: turns.aiTurnStart, aiTurnStop: turns.aiTurnStop, aiApply, putTool, skills, agents, aiModels, aiAccounts, app },
  };
});
vi.mock("../session", () => ({ reloadDefinitions }));

const { ApiError } = await import("../api");
const { ChatPanel } = await import("./ChatPanel");
const { newConversation, outcomesNow } = await import("./store");

const WCL = 'description = "Count tagged tasks."\nscript      = "count_tagged"\nfunction    = "run"\n';
const SCRIPT = 'fn run(params: Value) -> Result[Value, string] {\n    Ok(Value::Int(1))\n}\n';

const CREATE = {
  id: "toolu_c",
  kind: "tool",
  op: "create",
  name: "count_tagged",
  exists: false,
  before: null,
  after: WCL,
  script: "count_tagged",
  script_exists: false,
  script_before: null,
  script_after: SCRIPT,
  script_delete: false,
  problems: [],
  proposer: "assistant",
};

const CHANGE = {
  ...CREATE,
  id: "toolu_a",
  op: "change",
  exists: true,
  before: WCL,
  after: WCL.replace("Count tagged tasks.", "Count the tasks with a tag."),
  script_exists: true,
  script_before: SCRIPT,
  script_after: null,
};

const DELETE = { ...CREATE, id: "toolu_d", op: "delete", exists: true, before: WCL, after: null, script_after: null, script_before: SCRIPT, script_delete: true };

async function ask(m: ReturnType<typeof render>) {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Make a tool." } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
}

function propose(proposal: { id: string; name: string; op: string }) {
  const tool = proposal.op === "delete" ? "propose_tool_delete" : "propose_tool_change";
  live!.onEvent({ event: "tool", data: { id: proposal.id, name: tool, input: { name: proposal.name } } });
  live!.onEvent({ event: "tool_proposal", data: proposal });
  live!.onEvent({ event: "tool_result", data: { id: proposal.id, is_error: false, summary: { notes: [] } } });
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

describe("tool proposals", () => {
  it("shows a new tool's file and script, and applies it on the server", async () => {
    aiApply.mockResolvedValue({ op: "tool_create", tool_name: "count_tagged", script_name: "count_tagged" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CREATE);
    const card = await m.findByRole("region", { name: "Proposed new tool count_tagged" });
    expect(within(card).getByLabelText("Tool file")).toHaveTextContent('+ script = "count_tagged"');
    expect(within(card).getByLabelText("Script")).toHaveTextContent("+ fn run(params: Value)");
    expect(card).toHaveTextContent("scripts/count_tagged.ws (new)");
    expect(m.getByText("Checked tool count_tagged")).toBeInTheDocument();
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByText(/Applied\./)).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(CREATE);
    expect(putTool).not.toHaveBeenCalled();
    expect(within(card).getByRole("link", { name: "Open tool" })).toHaveAttribute("href", "/settings/tools/count_tagged");
    expect(outcomesNow()).toEqual(["[Applied by the user: created tool count_tagged]"]);
  });

  it("shows a change to the file alone, leaving the script out", async () => {
    aiApply.mockResolvedValue({ op: "tool_change", tool_name: "count_tagged" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to tool count_tagged" });
    expect(card).toHaveTextContent('- description = "Count tagged tasks."');
    expect(card).toHaveTextContent('+ description = "Count the tasks with a tag."');
    expect(within(card).queryByLabelText("Script")).toBeNull();
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(outcomesNow()).toEqual(["[Applied by the user: changed tool count_tagged]"]));
  });

  it("says a delete takes the script too when nothing else names it", async () => {
    aiApply.mockResolvedValue({ op: "tool_delete", tool_name: "count_tagged", script_name: "count_tagged" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(DELETE);
    const card = await m.findByRole("region", { name: "Proposed delete of tool count_tagged" });
    expect(card).toHaveTextContent("Its script scripts/count_tagged.ws is deleted too");
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(outcomesNow()).toEqual(["[Applied by the user: deleted tool count_tagged and its script count_tagged]"]));
  });

  it("does not apply a tool that changed since the proposal, and says so", async () => {
    aiApply.mockRejectedValue(new ApiError(409, { error: "stale", message: "the tool changed" }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to tool count_tagged" });
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByRole("alert")).toHaveTextContent("This tool changed since the proposal"));
    expect(within(card).queryByRole("button", { name: "Apply" })).toBeNull();
  });

  it("discards without writing", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CREATE);
    const card = await m.findByRole("region", { name: "Proposed new tool count_tagged" });
    fireEvent.click(within(card).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(within(card).getByText("Discarded.")).toBeInTheDocument());
    expect(aiApply).not.toHaveBeenCalled();
    expect(outcomesNow()).toEqual(["[Discarded by the user: new tool count_tagged]"]);
  });

  it("shows a tool change the AI made itself, linking to the tool", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    live!.onEvent({ event: "applied", data: { id: "toolu_x", op: "tool_change", tool_name: "count_tagged", script_name: null } });
    const card = await m.findByRole("region", { name: "Changed tool count_tagged" });
    expect(within(card).getByRole("link", { name: "count_tagged" })).toHaveAttribute("href", "/settings/tools/count_tagged");
  });

  it("shows how a run_script step went: its outcome, time, instructions, version and log", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    live!.onEvent({ event: "tool", data: { id: "toolu_r", name: "run_script", input: { code: "fn main() -> int { 1 }", function: null, args: null } } });
    live!.onEvent({
      event: "tool_result",
      data: {
        id: "toolu_r",
        is_error: false,
        summary: {
          notes: [],
          script: { tool: "run_script", outcome: "ok", ms: 12, instructions: 340, log: "hello", ref: { kind: "code", hash: "abcdef0123456789abcd" } },
        },
      },
    });
    const step = await m.findByText("Ran wscript code");
    const details = step.closest("details")!;
    await waitFor(() => expect(details).toHaveTextContent("(ok in 12 ms)"));
    expect(details).toHaveTextContent("340 instructions");
    expect(details).toHaveTextContent("version abcdef012345");
    expect(within(details as HTMLElement).getByLabelText("Log")).toHaveTextContent("hello");
    expect(within(details as HTMLElement).getByLabelText("Code")).toHaveTextContent("fn main() -> int { 1 }");
  });
});
