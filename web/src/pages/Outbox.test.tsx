import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Block, Device, DeviceCommand, Json, Note, Page, Row } from "../api";

// The Outbox page (outbox X7, X8, X14): three tabs, Draft, Ready and Error, each item's preview by
// kind with its status chip and an AI chip for an agent's draft, the actions its status allows,
// Waiting for phone, and New email, SMS and event.

const query = vi.fn();
const note = vi.fn();
const updateNote = vi.fn();
const createNote = vi.fn();
const deleteNote = vi.fn();
const devices = vi.fn();
const deviceCommands = vi.fn();
const assetUsage = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    api: { ...real.api, query, note, updateNote, createNote, deleteNote, devices, deviceCommands, assetUsage, restoreFromTrash: vi.fn() },
  };
});
const navigate = vi.fn();
let searchParams: Record<string, string> = {};
vi.mock("@solidjs/router", () => ({ useNavigate: () => navigate, useSearchParams: () => [searchParams, vi.fn()] }));

const { Outbox } = await import("./Outbox");

const body = (text: string): Block => ({ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text } }] });

/** The stored items by id. */
let items: Record<string, Note> = {};

function item(id: string, fields: Record<string, Json>, blocks: Block[] = []): Note {
  const n: Note = { id, type: "outbox", version: `v-${id}`, wcl: "", note: { fields, blocks } };
  items[id] = n;
  return n;
}

const row = (n: Note): Row => ({ id: n.id, type: n.type, title: String(n.note.fields.title ?? ""), fields: n.note.fields });

/** The items whose status the query names. */
function answer(q: string): Page {
  const statuses = [...q.matchAll(/status == "(\w+)"/g)].map((m) => m[1]);
  const rows = Object.values(items)
    .filter((n) => n.type === "outbox" && statuses.includes(String(n.note.fields.status)))
    .map(row);
  return { rows, next: null, total: rows.length };
}

const phone: Device = {
  id: "dev-pixel",
  name: "Pixel",
  platform: "android",
  capabilities: [],
  token: "device-pixel",
  push: false,
  last_seen: null,
  created: 0,
  settings: { commands: ["send_sms"], contacts_write: false, sms_send: true, exclusions: [], skip_pattern: "" },
};

const command = (id: string, status: DeviceCommand["status"]): DeviceCommand => ({
  id,
  device: "dev-pixel",
  name: "send_sms",
  payload: {},
  status,
  result: null,
  source: "handler:outbox_send",
});

beforeEach(() => {
  items = {};
  searchParams = {};
  item(
    "01AGENT",
    { title: "Re: Roof", kind: "email", status: "draft", drafted_by: "clerk", to: ["ada@example.com"], cc: ["bo@example.com"], subject: "Re: Roof", attachments: ["asset-1"] },
    [body("The roof is fixed.")],
  );
  item("01AGENTSMS", { title: "SMS to Bo", kind: "sms", status: "draft", drafted_by: "clerk", phone: "+15550100", device: "dev-pixel" }, [body("On my way")]);
  item("01DRAFT", { title: "New email", kind: "email", status: "draft", drafted_by: "user", to: [] }, [body("")]);
  item("01FAILED", { title: "Lunch", kind: "email", status: "failed", drafted_by: "user", error: "Reconnect Google in Settings > Integrations to send email." });
  item("01READY", { title: "Hello", kind: "email", status: "ready", drafted_by: "user" });
  item("01SMS", { title: "SMS to Cy", kind: "sms", status: "sending", drafted_by: "user", phone: "+15550111", device: "dev-pixel", sent_ref: "cmd-1" }, [body("Hi")]);
  item("01EVENTNEW", { title: "Retro", kind: "event", status: "draft", drafted_by: "user", location: "Room 2", when: { start: "2026-10-27", end: "2026-10-27" } });
  item("01EVENTCHG", { title: "Standup", kind: "event", status: "draft", drafted_by: "clerk", event: "01EVENT", location: "Room 9" });
  items["01EVENT"] = { id: "01EVENT", type: "event", version: "v-e", wcl: "", note: { fields: { title: "Standup", location: "Room 1" }, blocks: [] } };
  query.mockImplementation((q: string) => Promise.resolve(answer(q)));
  note.mockImplementation((id: string) => (items[id] ? Promise.resolve(items[id]) : Promise.reject(new Error("gone"))));
  updateNote.mockImplementation((id: string, doc: Note["note"]) => {
    items[id] = { ...items[id]!, note: doc, version: `${items[id]!.version}+` };
    return Promise.resolve(items[id]);
  });
  createNote.mockImplementation((type: string, doc: Note["note"]) => Promise.resolve({ id: "01NEW", type, version: "v1", wcl: "", note: doc }));
  deleteNote.mockResolvedValue({});
  devices.mockResolvedValue([phone]);
  deviceCommands.mockResolvedValue([command("cmd-1", "pending")]);
  assetUsage.mockResolvedValue({ filename: "roof.jpg" });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const card = (title: string) => {
  // The first link of the name is the item's own title; an event change also links its event.
  const link = screen.getAllByRole("link", { name: title })[0]!;
  return link.closest("li") as HTMLElement;
};

async function openTab(name: string) {
  fireEvent.click(screen.getByRole("tab", { name }));
  await waitFor(() => expect(screen.getByRole("tab", { name }).getAttribute("aria-selected")).toBe("true"));
}

describe("the Outbox page", () => {
  it("has three tabs, Draft, Ready and Error, each listing its items", async () => {
    render(() => <Outbox />);
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["Draft", "Ready", "Error"]);
    const expectTab = async (tab: string, titles: string[]) => {
      await openTab(tab);
      for (const t of titles) expect((await screen.findAllByRole("link", { name: t })).length).toBeGreaterThan(0);
      expect(within(screen.getByRole("list", { name: `${tab} items` })).getAllByRole("listitem").filter((li) => li.classList.contains("outbox-item"))).toHaveLength(titles.length);
    };
    await expectTab("Draft", ["Re: Roof", "SMS to Bo", "New email", "Retro", "Standup"]);
    await expectTab("Ready", ["Hello", "SMS to Cy"]);
    await expectTab("Error", ["Lunch"]);
    expect(query).toHaveBeenCalledWith('from outbox where status == "ready" || status == "sending" sort updated desc, id desc', null, 50);
    expect(screen.queryByRole("button", { name: "Approve all" })).toBeNull();
  });

  it("offers each status its actions", async () => {
    render(() => <Outbox />);
    const names = (title: string) =>
      within(card(title))
        .getAllByRole("button")
        .map((b) => b.textContent)
        .concat(within(card(title)).queryAllByRole("link", { name: "Edit" }).map(() => "Edit"));
    await screen.findByRole("link", { name: "Re: Roof" });
    // An agent's draft and the user's alike: nothing to approve separately.
    expect(names("Re: Roof").sort()).toEqual(["Delete", "Edit", "Ready"]);
    expect(names("New email").sort()).toEqual(["Delete", "Edit", "Ready"]);
    await openTab("Error");
    await screen.findByRole("link", { name: "Lunch" });
    expect(names("Lunch").sort()).toEqual(["Delete", "Edit", "Retry"]);
    expect(within(card("Lunch")).getByText("Reconnect Google in Settings > Integrations to send email.")).toBeTruthy();
    await openTab("Ready");
    await screen.findByRole("link", { name: "Hello" });
    expect(within(card("Hello")).queryAllByRole("button")).toHaveLength(0);
  });

  it("shows each item's status as a chip, with AI beside an agent's draft and no drafted-by line", async () => {
    render(() => <Outbox />);
    await screen.findByRole("link", { name: "Re: Roof" });
    const chips = (title: string) => [...card(title).querySelectorAll(".outbox-chip")].map((c) => c.textContent);
    expect(chips("Re: Roof")).toEqual(["Draft", "AI"]);
    expect(card("Re: Roof").querySelector(".outbox-ai-chip")?.getAttribute("title")).toBe("Drafted by clerk");
    expect(chips("New email")).toEqual(["Draft"]);
    expect(card("Re: Roof").textContent).not.toContain("Drafted by");
    expect(card("New email").textContent).not.toContain("Drafted by");
    await openTab("Ready");
    await screen.findByRole("link", { name: "Hello" });
    expect(chips("Hello")).toEqual(["Ready"]);
    await openTab("Error");
    await screen.findByRole("link", { name: "Lunch" });
    expect(chips("Lunch")).toEqual(["Error"]);
  });

  it("marks drafts ready, retries and deletes (an agent's rejected draft to the trash) by writing the item", async () => {
    render(() => <Outbox />);
    await screen.findByRole("link", { name: "Re: Roof" });
    fireEvent.click(within(card("Re: Roof")).getByRole("button", { name: "Ready" }));
    await waitFor(() => expect(items["01AGENT"]!.note.fields.status).toBe("ready"));
    expect(updateNote).toHaveBeenCalledWith("01AGENT", expect.anything(), "v-01AGENT");
    fireEvent.click(within(card("SMS to Bo")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deleteNote).toHaveBeenCalledWith("01AGENTSMS", "v-01AGENTSMS"));
    fireEvent.click(within(card("New email")).getByRole("button", { name: "Ready" }));
    await waitFor(() => expect(items["01DRAFT"]!.note.fields.status).toBe("ready"));
    await openTab("Error");
    await screen.findByRole("link", { name: "Lunch" });
    fireEvent.click(within(card("Lunch")).getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(items["01FAILED"]!.note.fields.status).toBe("ready"));
    expect(items["01FAILED"]!.note.fields.error).toBeUndefined();
  });

  it("previews an email's headers, body and attachments", async () => {
    render(() => <Outbox />);
    await screen.findByRole("link", { name: "Re: Roof" });
    const preview = await within(card("Re: Roof")).findByLabelText("Email preview");
    await waitFor(() => expect(preview.textContent).toContain("The roof is fixed."));
    expect(preview.textContent).toContain("ada@example.com");
    expect(preview.textContent).toContain("bo@example.com");
    expect(preview.textContent).toContain("Re: Roof");
    expect(await within(preview).findByRole("link", { name: "roof.jpg" })).toBeTruthy();
  });

  it("previews an SMS as a bubble with the number and the phone", async () => {
    render(() => <Outbox />);
    await screen.findByRole("link", { name: "SMS to Bo" });
    const preview = await within(card("SMS to Bo")).findByLabelText("SMS preview");
    expect(preview.querySelector(".outbox-bubble")?.textContent).toBe("On my way");
    await waitFor(() => expect(preview.textContent).toContain("To +15550100 · from Pixel"));
  });

  it("previews a change to an event as before and after, and a new event as its fields", async () => {
    render(() => <Outbox />);
    await screen.findAllByRole("link", { name: "Standup" });
    const change = await within(card("Standup")).findByLabelText("Event preview");
    await waitFor(() => expect(change.querySelector(".outbox-before")?.textContent).toBe("Room 1"));
    expect(change.querySelector(".outbox-after")?.textContent).toBe("Room 9");
    expect(change.textContent).toContain("A change to");
    await screen.findByRole("link", { name: "Retro" });
    const fresh = await within(card("Retro")).findByLabelText("Event preview");
    expect(fresh.textContent).toContain("A new event");
    expect(fresh.textContent).toContain("Room 2");
    expect(fresh.querySelector(".outbox-before")).toBeNull();
  });

  it("shows Waiting for phone while the SMS's command is pending", async () => {
    render(() => <Outbox />);
    await openTab("Ready");
    await screen.findByRole("link", { name: "SMS to Cy" });
    await waitFor(() => expect(card("SMS to Cy").querySelector(".outbox-status")?.textContent).toBe("Waiting for phone"));
    expect(deviceCommands).toHaveBeenCalledWith("dev-pixel");
  });

  it("shows Sending once the phone has the command done", async () => {
    deviceCommands.mockResolvedValue([command("cmd-1", "done")]);
    render(() => <Outbox />);
    await openTab("Ready");
    await screen.findByRole("link", { name: "SMS to Cy" });
    await waitFor(() => expect(deviceCommands).toHaveBeenCalled());
    await Promise.resolve();
    expect(card("SMS to Cy").querySelector(".outbox-status")?.textContent).toBe("Sending");
  });

  it("opens on the item a link names, in the tab of its status", async () => {
    searchParams = { item: "01FAILED" };
    render(() => <Outbox />);
    await waitFor(() => expect(screen.getByRole("tab", { name: "Error" }).getAttribute("aria-selected")).toBe("true"));
    await screen.findByRole("link", { name: "Lunch" });
    expect(card("Lunch").getAttribute("aria-current")).toBe("true");
  });

  it("opens on the tab ?tab= names, as SMS Reply all and the drafts notification open Draft", async () => {
    searchParams = { tab: "error" };
    render(() => <Outbox />);
    await waitFor(() => expect(screen.getByRole("tab", { name: "Error" }).getAttribute("aria-selected")).toBe("true"));
    await screen.findByRole("link", { name: "Lunch" });
    cleanup();
    // The older link to Drafts opens Draft.
    searchParams = { tab: "drafts" };
    render(() => <Outbox />);
    await waitFor(() => expect(screen.getByRole("tab", { name: "Draft" }).getAttribute("aria-selected")).toBe("true"));
    await screen.findByRole("link", { name: "New email" });
  });

  for (const [button, kind, title] of [
    ["New email", "email", "New email"],
    ["New SMS", "sms", "New SMS"],
    ["New event", "event", "New event"],
  ] as const) {
    it(`${button} creates the user's draft and opens it`, async () => {
      render(() => <Outbox />);
      fireEvent.click(screen.getByRole("button", { name: button }));
      await waitFor(() => expect(navigate).toHaveBeenCalledWith("/notes/01NEW"));
      expect(createNote).toHaveBeenCalledWith("outbox", expect.objectContaining({ fields: { title, kind, status: "draft", drafted_by: "user" } }));
    });
  }
});
