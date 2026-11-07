import { cleanup, fireEvent, render, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";
import { when } from "../admin/format";
import { turnsOver } from "./fakeTurns";

// The chat panel's scheduled task proposal card (TV-71): the schedule in words and the next runs,
// the hosts, secrets, timeout and retries, the warnings, the task file's diff and the script's when
// the proposal writes it, and what else runs the script; Apply sends it to POST /api/ai/apply, a
// task changed since the proposal is not applied, Discard writes nothing, a delete says the script
// and run history stay, and the outcome reaches the next message.

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
    api: { ...real.api, aiTurnStart: turns.aiTurnStart, aiTurnStop: turns.aiTurnStop, aiApply, skills, agents, aiModels, aiAccounts, app },
  };
});
vi.mock("../session", () => ({ reloadDefinitions }));

const { ApiError } = await import("../api");
const { ChatPanel } = await import("./ChatPanel");
const { newConversation, outcomesNow } = await import("./store");
const { taskGrants } = await import("./TaskProposalCard");

const AFTER = 'label = "Nightly"\nschedule = "0 3 * * *"\ntimezone = "Europe/London"\nscript = "stamp"\nallow_domains = ["api.example.com"]\nsecrets = ["api_key"]\n';
const BEFORE = AFTER.replace("0 3 * * *", "0 2 * * *");
const NEXT = [1_767_582_000_000, 1_767_668_400_000, 1_767_754_800_000];
const SECRETS_WARNING = "The task nightly may read the secrets api_key and reach api.example.com: it could send those secrets there.";

const TASK = {
  label: "Nightly",
  description: null,
  schedule: "0 3 * * *",
  schedule_text: "Every day at 03:00 (Europe/London)",
  timezone: "Europe/London",
  script: "stamp",
  function: "run",
  allow_domains: ["api.example.com"],
  secrets: ["api_key"],
  read_paths: [],
  write_paths: [],
  retries: 3,
  retry_delay_ms: 60_000,
  timeout_ms: 30_000,
  paused: false,
};

const OTHER = { kind: "handler", name: "item", file: "types/item.wcl", event: "after_update", allow_domains: [], secrets: [], read_paths: [], write_paths: [], schedule: null, timezone: null, paused: false };

const CREATE = {
  id: "toolu_t",
  kind: "task",
  op: "create",
  name: "nightly",
  exists: false,
  before: null,
  after: AFTER,
  script: "stamp",
  script_exists: true,
  script_before: null,
  script_after: null,
  task: TASK,
  next_runs: NEXT,
  runs_as: [OTHER],
  warnings: [SECRETS_WARNING],
  proposer: "assistant",
};

const SCRIPT = "fn run() -> Result[(), string] {\n    Ok(())\n}\n";
const CHANGE = { ...CREATE, id: "toolu_c", op: "change", exists: true, before: BEFORE, script_before: SCRIPT, script_after: SCRIPT.replace("Ok(())", "let x = 1\n    Ok(())"), warnings: [] };
const READ_ONLY = { ...CREATE, id: "toolu_r", script_before: SCRIPT };
const PAUSED = { ...CREATE, id: "toolu_p", task: { ...TASK, paused: true }, next_runs: [] };
const DELETE = { ...CREATE, id: "toolu_d", op: "delete", exists: true, before: AFTER, after: null, next_runs: [], runs_as: [], warnings: [] };

async function ask(m: ReturnType<typeof render>) {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Schedule it." } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
}

function propose(proposal: { id: string; name: string; op: string }) {
  const tool = proposal.op === "delete" ? "propose_task_delete" : "propose_task_change";
  live!.onEvent({ event: "tool", data: { id: proposal.id, name: tool, input: { name: proposal.name } } });
  live!.onEvent({ event: "task_proposal", data: proposal });
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

describe("task proposals", () => {
  it("shows the schedule, the next runs, the grants and the warning, and applies it", async () => {
    aiApply.mockResolvedValue({ op: "task_create", task_name: "nightly", script_name: null, next_run: NEXT[0] });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CREATE);
    const card = await m.findByRole("region", { name: "Proposed new task nightly" });
    expect(m.getByText("Checked task nightly")).toBeInTheDocument();
    expect(card).toHaveTextContent("tasks/nightly.wcl (new)");
    expect(card).toHaveTextContent("Every day at 03:00 (Europe/London)");
    const next = within(card).getByLabelText("Next runs");
    expect(within(next).getAllByRole("listitem").map((li) => li.textContent)).toEqual(NEXT.map(when));
    const grants = within(card).getByLabelText("Grants");
    expect(grants).toHaveTextContent("Calls run in scripts/stamp.ws");
    expect(grants).toHaveTextContent("Hosts: api.example.com");
    expect(grants).toHaveTextContent("Secrets: api_key");
    expect(grants).toHaveTextContent("Timeout: 30 s");
    expect(within(card).getByLabelText("Warnings")).toHaveTextContent(SECRETS_WARNING);
    expect(within(card).getByLabelText("Task file")).toHaveTextContent('+ schedule = "0 3 * * *"');
    expect(within(card).queryByLabelText("Script")).toBeNull();
    expect(within(card).getByLabelText("Runs as")).toHaveTextContent("Handler after_update of type item");
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByText(/Applied\./)).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(CREATE);
    expect(within(card).getByRole("link", { name: "Open task" })).toHaveAttribute("href", "/scheduled/nightly");
    expect(outcomesNow()).toEqual(["[Applied by the user: created task nightly]"]);
  });

  it("shows the script diff of a change, and the shared script's other users", async () => {
    aiApply.mockResolvedValue({ op: "task_change", task_name: "nightly", script_name: "stamp", next_run: NEXT[0] });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to task nightly" });
    expect(within(card).getByLabelText("Task file")).toHaveTextContent('- schedule = "0 2 * * *"');
    expect(within(card).getByLabelText("Script")).toHaveTextContent("+ let x = 1");
    expect(card).toHaveTextContent("scripts/stamp.ws also runs as these, and the script change applies to each");
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(outcomesNow()).toEqual(["[Applied by the user: changed task nightly]"]));
  });

  it("shows the script a proposal does not write, read-only", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(READ_ONLY);
    const card = await m.findByRole("region", { name: "Proposed new task nightly" });
    expect(card).toHaveTextContent("scripts/stamp.ws stays as it is; the task runs it:");
    expect(within(card).getByLabelText("Script")).toHaveTextContent("fn run() -> Result[(), string]");
    expect(within(card).getByLabelText("Script")).not.toHaveTextContent("+ ");
  });

  it("says a paused task runs only by hand", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(PAUSED);
    const card = await m.findByRole("region", { name: "Proposed new task nightly" });
    expect(card).toHaveTextContent("Paused: it runs only when run by hand.");
    expect(within(card).queryByLabelText("Next runs")).toBeNull();
  });

  it("applies a delete, which keeps the script and the run history", async () => {
    aiApply.mockResolvedValue({ op: "task_delete", task_name: "nightly", script_name: null, next_run: null });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(DELETE);
    const card = await m.findByRole("region", { name: "Proposed delete of task nightly" });
    expect(within(card).getByLabelText("Task file")).toHaveTextContent('- schedule = "0 3 * * *"');
    expect(card).toHaveTextContent("Its script scripts/stamp.ws stays, and so does its run history.");
    expect(within(card).queryByLabelText("Grants")).toBeNull();
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(outcomesNow()).toEqual(["[Applied by the user: deleted task nightly]"]));
    expect(within(card).queryByRole("link", { name: "Open task" })).toBeNull();
  });

  it("does not apply a task that changed since the proposal, and says so", async () => {
    aiApply.mockRejectedValue(new ApiError(409, { error: "stale", message: "the task changed" }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to task nightly" });
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByRole("alert")).toHaveTextContent("This task or its script changed since the proposal"));
    expect(within(card).queryByRole("button", { name: "Apply" })).toBeNull();
    expect(outcomesNow()[0]).toMatch(/^\[Failed to apply: change to task nightly: /);
  });

  it("discards without writing", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CREATE);
    const card = await m.findByRole("region", { name: "Proposed new task nightly" });
    fireEvent.click(within(card).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(within(card).getByText("Discarded.")).toBeInTheDocument());
    expect(aiApply).not.toHaveBeenCalled();
    expect(outcomesNow()).toEqual(["[Discarded by the user: new task nightly]"]);
  });
});

describe("the grants block", () => {
  it("names any host, no secrets and no retries", () => {
    expect(taskGrants({ ...TASK, allow_domains: ["*"], secrets: [], retries: 0 })).toEqual([
      "Calls run in scripts/stamp.ws",
      "Hosts: any host, including your local network",
      "Secrets: none",
      "Timeout: 30 s",
      "Retries: none",
    ]);
  });
});
