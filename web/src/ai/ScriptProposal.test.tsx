import { cleanup, fireEvent, render, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../api";
import { turnsOver } from "./fakeTurns";

// The chat panel's saved script proposal card (TV-70): the script's diff, the "Runs as" list of
// each handler, task and tool with its grants, a warning for a run that reaches any host; Apply
// sends it to POST /api/ai/apply, a script changed since the proposal is not applied, Discard
// writes nothing, and the outcome reaches the next message.

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

const { RunsAs, grantsOf, userLabel } = await import("./RunsAs");

const BEFORE = "fn run() -> Result[(), string] {\n    Ok(())\n}\n";
const AFTER = "fn run() -> Result[(), string] {\n    let x = 1\n    Ok(())\n}\n";

const TASK = { kind: "task", name: "nightly", file: "tasks/nightly.wcl", event: null, allow_domains: ["api.example.com"], secrets: ["api_key"], read_paths: [], write_paths: [], schedule: "0 3 * * *", timezone: "Europe/London", paused: true };
const HANDLER = { kind: "handler", name: "item", file: "types/item.wcl", event: "after_update", allow_domains: ["*"], secrets: [], read_paths: [], write_paths: [], schedule: null, timezone: null, paused: false };
const WARNING = "The after_update handler of type item may reach any host (allow_domains = [\"*\"]), including your local network and this server.";

const CHANGE = {
  id: "toolu_s",
  kind: "script",
  op: "change",
  name: "stamp",
  exists: true,
  before: BEFORE,
  after: AFTER,
  runs_as: [HANDLER, TASK],
  warnings: [WARNING],
  proposer: "assistant",
};

const CREATE = { ...CHANGE, id: "toolu_c", op: "create", name: "fresh", exists: false, before: null, runs_as: [], warnings: [] };
const DELETE = { ...CHANGE, id: "toolu_d", op: "delete", name: "old", after: null, runs_as: [], warnings: [] };

async function ask(m: ReturnType<typeof render>) {
  fireEvent.input(m.getByLabelText("Ask AI"), { target: { value: "Change the script." } });
  fireEvent.click(m.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(live).not.toBeNull());
}

function propose(proposal: { id: string; name: string; op: string; device?: unknown }) {
  const tool = proposal.op === "delete" ? "propose_script_delete" : "propose_script_change";
  live!.onEvent({ event: "tool", data: { id: proposal.id, name: tool, input: { name: proposal.name } } });
  live!.onEvent({ event: "script_proposal", data: proposal });
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

describe("script proposals", () => {
  it("shows and applies an atomic Android device file proposal", async () => {
    aiApply.mockResolvedValue({ op: "device_file_change", platform: "android" });
    const m = render(() => <ChatPanel />);
    await ask(m);
    const proposal = {
      id: "phone_bundle", kind: "device_file", platform: "android", before: "", after: "handler sms_received {}",
      requires_admin: true, proposer: "assistant", devices: [{ id: "phone", name: "My phone", settings: '{"commands":["send_sms"]}', health: null }],
      snapshots: [{ ...CHANGE, device: { note: "Runs on your phone at its next sync, with no further prompt there.", platforms: ["android"], devices: [], can: [{ key: "sms_read", label: "Read all SMS, including codes", new: true }] } }],
    };
    live!.onEvent({ event: "device_file_proposal", data: proposal });
    live!.end();
    const card = await waitFor(() => m.getByRole("region", { name: "Proposed Android device file" }));
    expect(card.textContent).toContain("signed-in admin");
    expect(card.textContent).toContain("My phone");
    expect(card.textContent).toContain("send_sms");
    expect(card.textContent).toContain("New: Read all SMS");
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(aiApply).toHaveBeenCalledWith(proposal));
    await waitFor(() => expect(card.textContent).toContain("Applied."));
  });

  it("discards a device file proposal without writing", async () => {
    const m = render(() => <ChatPanel />);
    await ask(m);
    live!.onEvent({ event: "device_file_proposal", data: { id: "discard_phone", kind: "device_file", platform: "android", before: "", after: "", snapshots: [], devices: [], requires_admin: false } });
    live!.end();
    const card = await waitFor(() => m.getByRole("region", { name: "Proposed Android device file" }));
    fireEvent.click(within(card).getByRole("button", { name: "Discard" }));
    expect(card.textContent).toContain("Discarded.");
    expect(aiApply).not.toHaveBeenCalled();
  });

  it("shows the diff, what runs the script with its grants and the any-host warning, and applies it", async () => {
    aiApply.mockResolvedValue({ op: "script_change", script_name: "stamp" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to script stamp" });
    expect(within(card).getByLabelText("Script")).toHaveTextContent("+ let x = 1");
    expect(card).toHaveTextContent("scripts/stamp.ws");
    expect(m.getByText("Checked script stamp")).toBeInTheDocument();
    const runs = within(card).getByLabelText("Runs as");
    expect(runs).toHaveTextContent("Handler after_update of type item");
    expect(runs).toHaveTextContent("Hosts: any host, including your local network");
    expect(runs).toHaveTextContent("Task nightly");
    expect(runs).toHaveTextContent("Schedule: 0 3 * * * (Europe/London), paused");
    expect(runs).toHaveTextContent("Hosts: api.example.com");
    expect(runs).toHaveTextContent("Secrets: api_key");
    expect(within(card).getByLabelText("Warnings")).toHaveTextContent(WARNING);
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByText(/Applied\./)).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(CHANGE);
    expect(within(card).getByRole("link", { name: "Open script" })).toHaveAttribute("href", "/settings/scripts/stamp");
    expect(outcomesNow()).toEqual(["[Applied by the user: changed script stamp]"]);
  });

  it("says nothing runs a new script yet", async () => {
    aiApply.mockResolvedValue({ op: "script_create", script_name: "fresh" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CREATE);
    const card = await m.findByRole("region", { name: "Proposed new script fresh" });
    expect(card).toHaveTextContent("scripts/fresh.ws (new)");
    expect(card).toHaveTextContent("Nothing runs this script yet.");
    expect(within(card).queryByLabelText("Warnings")).toBeNull();
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(outcomesNow()).toEqual(["[Applied by the user: created script fresh]"]));
  });

  it("applies a delete", async () => {
    aiApply.mockResolvedValue({ op: "script_delete", script_name: "old" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(DELETE);
    const card = await m.findByRole("region", { name: "Proposed delete of script old" });
    expect(within(card).getByLabelText("Script")).toHaveTextContent("- fn run()");
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(outcomesNow()).toEqual(["[Applied by the user: deleted script old]"]));
    expect(within(card).queryByRole("link", { name: "Open script" })).toBeNull();
  });

  it("does not apply a script that changed since the proposal, and says so", async () => {
    aiApply.mockRejectedValue(new ApiError(409, { error: "stale", message: "the script changed" }));
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to script stamp" });
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByRole("alert")).toHaveTextContent("This script changed since the proposal"));
    expect(within(card).queryByRole("button", { name: "Apply" })).toBeNull();
    expect(outcomesNow()[0]).toMatch(/^\[Failed to apply: change to script stamp: /);
  });

  it("shows that a phone script runs on the phone, on which devices with their switches, and what is new that it can do", async () => {
    const PHONE = {
      ...CHANGE,
      id: "toolu_p",
      name: "device_messages",
      runs_as: [{ kind: "device", name: "android", file: "devices/android.wcl", event: "command send_sms", allow_domains: [], secrets: [], read_paths: [], write_paths: [], schedule: null, timezone: null, paused: false }],
      warnings: [],
      device: {
        note: "Runs on your phone at its next sync, with no further prompt there.",
        platforms: ["android"],
        devices: [
          { id: "d1", name: "Pixel", platform: "android", sms_send: true, contacts_write: false },
          { id: "d2", name: "Tablet", platform: "android", sms_send: false, contacts_write: true },
        ],
        can: [
          { key: "sms_send", label: "Send SMS", new: false },
          { key: "sms_read", label: "Read all SMS, including the codes and people that capture skips", new: true },
        ],
      },
    };
    aiApply.mockResolvedValue({ op: "script_change", script_name: "device_messages" });
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(PHONE);
    const card = await m.findByRole("region", { name: "Proposed change to script device_messages" });
    expect(within(card).getByRole("note")).toHaveTextContent("Runs on your phone at its next sync, with no further prompt there.");
    const on = within(card).getByLabelText("Runs on");
    expect(on).toHaveTextContent("Pixel android: SMS send on, contacts write off");
    expect(on).toHaveTextContent("Tablet android: SMS send off, contacts write on");
    const can = within(card).getByLabelText("This code can");
    const items = within(can).getAllByRole("listitem");
    expect(items.map((i) => i.textContent)).toEqual(["Send SMS", "New: Read all SMS, including the codes and people that capture skips"]);
    expect(items[1]).toHaveClass("ai-new");
    expect(items[0]).not.toHaveClass("ai-new");
    expect(within(card).getByLabelText("Runs as")).toHaveTextContent("Device file android: command send_sms");
    fireEvent.click(within(card).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(within(card).getByText(/Applied\./)).toBeInTheDocument());
    expect(aiApply).toHaveBeenCalledWith(PHONE);
  });

  it("says when no device of the platform is paired, and shows no phone block for other scripts", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose({ ...CHANGE, id: "toolu_n", name: "device_calls", device: { note: "Runs on your phone at its next sync, with no further prompt there.", platforms: ["android"], devices: [], can: [] } });
    const card = await m.findByRole("region", { name: "Proposed change to script device_calls" });
    expect(card).toHaveTextContent("No android device is paired yet.");
    expect(card).toHaveTextContent("None of: send SMS, read SMS, read calls, add or delete contacts, show notifications, write notes.");
    propose({ ...CHANGE, id: "toolu_o" });
    const other = await m.findByRole("region", { name: "Proposed change to script stamp" });
    expect(within(other).queryByLabelText("Runs on")).toBeNull();
    expect(other).not.toHaveTextContent("Runs on your phone");
  });

  it("discards without writing", async () => {
    const m = render(() => <ChatPanel noteId={() => ""} />);
    await ask(m);
    propose(CHANGE);
    const card = await m.findByRole("region", { name: "Proposed change to script stamp" });
    fireEvent.click(within(card).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(within(card).getByText("Discarded.")).toBeInTheDocument());
    expect(aiApply).not.toHaveBeenCalled();
    expect(outcomesNow()).toEqual(["[Discarded by the user: change to script stamp]"]);
  });
});

describe("the Runs as list", () => {
  it("labels each kind of user and its grants", () => {
    expect(userLabel({ ...TASK, kind: "device", name: "android", event: "command send_sms" } as never)).toBe("Device file android: command send_sms");
    expect(userLabel({ ...TASK, kind: "tool", name: "echo" } as never)).toBe("Tool echo");
    expect(grantsOf({ ...TASK, schedule: null, allow_domains: [], secrets: [], read_paths: ["/data/in"], write_paths: ["/data/out"] } as never)).toEqual(["Reads: /data/in", "Writes: /data/out"]);
    expect(grantsOf({ ...TASK, kind: "tool", schedule: null, allow_domains: [], secrets: [] } as never)).toEqual(["No hosts, secrets or paths"]);
  });

  it("shows nothing for no users unless told what to say", () => {
    const m = render(() => <RunsAs uses={[]} warnings={[]} heading="Runs as" />);
    expect(m.queryByLabelText("Runs as")).toBeNull();
    expect(m.container).toHaveTextContent("");
  });
});
