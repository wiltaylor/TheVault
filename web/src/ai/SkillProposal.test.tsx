import { cleanup, fireEvent, render, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";
import { turnsOver } from "./fakeTurns";

// The chat panel's skill proposal card (skills S9): the SKILL.md or file diff, a binary file by
// size and type; Apply sends it to POST /api/ai/apply and the skill list reads again, a skill
// changed since the proposal is not applied, a refusal shows, Discard writes nothing, Apply all
// includes it, the outcome reaches the next message, and a change the AI made itself links to
// the skill's screen.

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
const putSkill = vi.fn();
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
    api: { ...real.api, aiTurnStart: turns.aiTurnStart, aiTurnStop: turns.aiTurnStop, aiApply, putSkill, skills, agents, aiModels, aiAccounts, app },
  };
});
vi.mock("../session", () => ({ reloadDefinitions }));

const { ApiError } = await import("../api");
const { ChatPanel } = await import("./ChatPanel");
const { newConversation, outcomesNow } = await import("./store");

const BEFORE = "---\nname: pdf-tools\ndescription: Fills PDF forms.\n---\n\nRun the script.\n";
const AFTER = BEFORE.replace("Run the script.", "Run scripts/fill.py.");

const CREATE = {
  id: "toolu_c",
  kind: "skill",
  op: "create",
  name: "pdf-tools",
  path: "SKILL.md",
  exists: false,
  before: null,
  after: BEFORE,
  issues: [],
  base: null,
  proposer: "assistant",
};

const CHANGE = { ...CREATE, id: "toolu_a", op: "change", exists: true, before: BEFORE, after: AFTER, base: "abc" };

const BINARY = {
  id: "toolu_b",
  kind: "skill",
  op: "file",
  name: "pdf-tools",
  path: "assets/blank.pdf",
  exists: false,
  skill_exists: true,
  binary: true,
  size: 2048,
  media_type: "application/pdf",
  before: null,
  after: null,
  before_size: null,
  before_media_type: null,
  base64: "AAEC",
  issues: [],
  base: null,
  proposer: "assistant",
};

const DELETE = { id: "toolu_d", kind: "skill", op: "delete", name: "old-skill", exists: true, files: ["SKILL.md"], before: "---\nname: old-skill\n---\n", after: null, issues: [], base: "x" };

const TOOLS: Record<string, string> = {
  create: "propose_skill_change",
  change: "propose_skill_change",
  file: "propose_skill_file_change",
  file_delete: "propose_skill_file_delete",
  delete: "propose_skill_delete",
};

async function ask(m: ReturnType<typeof render>) {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Make a PDF skill." } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
}

function propose(proposal: { id: string; name: string; op: string; path?: string }) {
  live!.onEvent({ event: "tool", data: { id: proposal.id, name: TOOLS[proposal.op], input: { name: proposal.name, path: proposal.path ?? "" } } });
  live!.onEvent({ event: "skill_proposal", data: proposal });
  live!.onEvent({ event: "tool_result", data: { id: proposal.id, is_error: false, summary: { notes: [], skills: [proposal.name] } } });
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

describe("skill proposals", () => {
  it("shows a new skill's SKILL.md, applies it on the server and reads the skills again", async () => {
    aiApply.mockResolvedValue({ op: "skill_create", skill_name: "pdf-tools" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CREATE);
    const card = await m.findByRole("region", { name: "Proposed new skill pdf-tools" });
    expect(card).toHaveTextContent("+ name: pdf-tools");
    expect(card).toHaveTextContent("+ Run the script.");
    expect(m.getByText("Checked SKILL.md of skill pdf-tools")).toBeInTheDocument();
    skills.mockClear();
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByText(/Applied\./)).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(CREATE);
    expect(putSkill).not.toHaveBeenCalled();
    expect(within(card).getByRole("link", { name: "Open skill" })).toHaveAttribute("href", "/skills/pdf-tools");
    await waitFor(() => expect(skills).toHaveBeenCalled());
    expect(outcomesNow()).toEqual(["[Applied by the user: created skill pdf-tools]"]);
  });

  it("shows a change as the lines removed and added, linking to the skill", async () => {
    aiApply.mockResolvedValue({ op: "skill_change", skill_name: "pdf-tools" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to SKILL.md of skill pdf-tools" });
    expect(card).toHaveTextContent("- Run the script.");
    expect(card).toHaveTextContent("+ Run scripts/fill.py.");
    expect(within(card).getByRole("link", { name: "pdf-tools" })).toHaveAttribute("href", "/skills/pdf-tools");
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(outcomesNow()).toEqual(["[Applied by the user: changed skill pdf-tools]"]));
  });

  it("shows a binary file by its size and type", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(BINARY);
    const card = await m.findByRole("region", { name: "Proposed new file assets/blank.pdf in skill pdf-tools" });
    expect(card).toHaveTextContent("2.0 KB, application/pdf");
    expect(card.querySelector("pre")).toBeNull();
  });

  it("does not apply a skill that changed since the proposal, and says so", async () => {
    aiApply.mockRejectedValue(new ApiError(409, { error: "stale", message: "the skill pdf-tools changed since the proposal" }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to SKILL.md of skill pdf-tools" });
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByRole("alert")).toHaveTextContent("This skill changed since the proposal"));
    expect(within(card).queryByRole("button", { name: "Apply" })).toBeNull();
    expect(outcomesNow()[0]).toMatch(/^\[Failed to apply: change to skill pdf-tools: This skill changed/);
  });

  it("shows why the server refused it", async () => {
    aiApply.mockRejectedValue(new ApiError(403, { error: "needs_admin", message: "Apply skill changes in the web app." }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CREATE);
    const card = await m.findByRole("region", { name: "Proposed new skill pdf-tools" });
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByRole("alert")).toHaveTextContent("Apply skill changes in the web app."));
    expect(within(card).getByRole("button", { name: "Apply" })).toBeInTheDocument();
  });

  it("discards without writing", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CREATE);
    const card = await m.findByRole("region", { name: "Proposed new skill pdf-tools" });
    fireEvent.click(within(card).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(within(card).getByText("Discarded.")).toBeInTheDocument());
    expect(aiApply).not.toHaveBeenCalled();
    expect(outcomesNow()).toEqual(["[Discarded by the user: new skill pdf-tools]"]);
  });

  it("Apply all applies a new skill, its file and a delete in order", async () => {
    aiApply
      .mockResolvedValueOnce({ op: "skill_create", skill_name: "pdf-tools" })
      .mockResolvedValueOnce({ op: "skill_file", skill_name: "pdf-tools", path: "assets/blank.pdf" })
      .mockResolvedValueOnce({ op: "skill_delete", skill_name: "old-skill" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CREATE);
    propose(BINARY);
    propose(DELETE);
    const deleted = await m.findByRole("region", { name: "Proposed delete of skill old-skill" });
    expect(deleted).toHaveTextContent("- name: old-skill");
    fireEvent.click(m.getByRole("button", { name: "Apply all (3)" }));
    await waitFor(() => expect(aiApply).toHaveBeenCalledTimes(3));
    expect(aiApply).toHaveBeenNthCalledWith(1, CREATE);
    expect(aiApply).toHaveBeenNthCalledWith(2, BINARY);
    expect(aiApply).toHaveBeenNthCalledWith(3, DELETE);
    await waitFor(() => expect(within(deleted).getByText(/Applied\./)).toBeInTheDocument());
    expect(outcomesNow()).toEqual([
      "[Applied by the user: created skill pdf-tools]",
      "[Applied by the user: wrote assets/blank.pdf in skill pdf-tools]",
      "[Applied by the user: deleted skill old-skill]",
    ]);
  });

  it("shows a change the AI made itself, linking to the skill, and reads the skills again", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    skills.mockClear();
    live!.onEvent({ event: "applied", data: { id: "toolu_c", op: "skill_create", skill_name: "pdf-tools" } });
    const card = await m.findByRole("region", { name: "Created skill pdf-tools" });
    expect(within(card).getByRole("link", { name: "pdf-tools" })).toHaveAttribute("href", "/skills/pdf-tools");
    await waitFor(() => expect(skills).toHaveBeenCalled());
  });
});
