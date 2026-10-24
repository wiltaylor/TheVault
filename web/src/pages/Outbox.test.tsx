import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Block, Device, DeviceCommand, Json, Note, Page, Row } from "../api";

// The Outbox page (outbox X7, X8): a tab per status, each item's preview by kind and the actions
// its status allows, Approve all after a confirm, Waiting for phone, and New email, SMS and event.

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
    "01REVIEW",
    { title: "Re: Roof", kind: "email", status: "review", drafted_by: "clerk", to: ["ada@example.com"], cc: ["bo@example.com"], subject: "Re: Roof", attachments: ["asset-1"] },
    [body("The roof is fixed.")],
  );
  item("01REVIEW2", { title: "SMS to Bo", kind: "sms", status: "review", drafted_by: "clerk", phone: "+15550100", device: "dev-pixel" }, [body("On my way")]);
  item("01DRAFT", { title: "New email", kind: "email", status: "draft", drafted_by: "user", to: [] }, [body("")]);
  item("01FAILED", { title: "Lunch", kind: "email", status: "failed", drafted_by: "user", error: "Reconnect Google in Settings > Integrations to send email." });
  item("01SENT", { title: "Hello", kind: "email", status: "sent", drafted_by: "user", result: "01COPY" });
  item("01REJECTED", { title: "Spam", kind: "email", status: "rejected", drafted_by: "clerk" });
  item("01SMS", { title: "SMS to Cy", kind: "sms", status: "sending", drafted_by: "user", phone: "+15550111", device: "dev-pixel", sent_ref: "cmd-1" }, [body("Hi")]);
  item("01EVENTNEW", { title: "Retro", kind: "event", status: "draft", drafted_by: "user", location: "Room 2", when: { start: "2026-10-27", end: "2026-10-27" } });
  item("01EVENTCHG", { title: "Standup", kind: "event", status: "review", drafted_by: "clerk", event: "01EVENT", location: "Room 9" });
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
  const link = screen.getByRole("link", { name: title });
  return link.closest("li") as HTMLElement;
};

async function openTab(name: string) {
  fireEvent.click(screen.getByRole("tab", { name }));
  await waitFor(() => expect(screen.getByRole("tab", { name }).getAttribute("aria-selected")).toBe("true"));
}

describe("the Outbox page", () => {
  it("has a tab per status, each listing its items", async () => {
    render(() => <Outbox />);
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["Review", "Drafts", "Sending", "Sent", "Failed", "Rejected"]);
    await screen.findByRole("link", { name: "Re: Roof" });
    expect(screen.getByRole("link", { name: "SMS to Bo" })).toBeTruthy();
    expect(screen.queryByRole("link", { name: "New email" })).toBeNull();
    const expectTab = async (tab: string, titles: string[]) => {
      await openTab(tab);
      for (const t of titles) expect(await screen.findByRole("link", { name: t })).toBeTruthy();
      expect(within(screen.getByRole("list")).getAllByRole("listitem").filter((li) => li.classList.contains("outbox-item"))).toHaveLength(titles.length);
    };
    await expectTab("Drafts", ["New email", "Retro"]);
    await expectTab("Sending", ["SMS to Cy"]);
    await expectTab("Sent", ["Hello"]);
    await expectTab("Failed", ["Lunch"]);
    await expectTab("Rejected", ["Spam"]);
    expect(query).toHaveBeenCalledWith('from outbox where status == "approved" || status == "sending" sort updated desc, id desc', null, 50);
  });

  it("offers each status its actions", async () => {
    render(() => <Outbox />);
    const names = (title: string) =>
      within(card(title))
        .getAllByRole("button")
        .map((b) => b.textContent)
        .concat(within(card(title)).queryAllByRole("link", { name: "Edit" }).map(() => "Edit"));
    await screen.findByRole("link", { name: "Re: Roof" });
    expect(names("Re: Roof").sort()).toEqual(["Approve and send", "Edit", "Reject"]);
    await openTab("Drafts");
    await screen.findByRole("link", { name: "New email" });
    expect(names("New email").sort()).toEqual(["Delete", "Edit", "Ready"]);
    await openTab("Failed");
    await screen.findByRole("link", { name: "Lunch" });
    expect(names("Lunch").sort()).toEqual(["Edit", "Retry"]);
    expect(within(card("Lunch")).getByText("Reconnect Google in Settings > Integrations to send email.")).toBeTruthy();
    await openTab("Rejected");
    await screen.findByRole("link", { name: "Spam" });
    expect(names("Spam").sort()).toEqual(["Delete", "Edit"]);
    await openTab("Sent");
    await screen.findByRole("link", { name: "Hello" });
    expect(within(card("Hello")).queryAllByRole("button")).toHaveLength(0);
    expect(within(card("Hello")).getByRole("link", { name: /Sent copy/ }).getAttribute("href")).toBe("/notes/01COPY");
  });

  it("approves, sends, rejects, retries and deletes by writing the item", async () => {
    render(() => <Outbox />);
    await screen.findByRole("link", { name: "Re: Roof" });
    fireEvent.click(within(card("Re: Roof")).getByRole("button", { name: "Approve and send" }));
    await waitFor(() => expect(items["01REVIEW"]!.note.fields.status).toBe("approved"));
    expect(updateNote).toHaveBeenCalledWith("01REVIEW", expect.anything(), "v-01REVIEW");
    fireEvent.click(within(card("SMS to Bo")).getByRole("button", { name: "Reject" }));
    await waitFor(() => expect(items["01REVIEW2"]!.note.fields.status).toBe("rejected"));
    await openTab("Drafts");
    await screen.findByRole("link", { name: "New email" });
    fireEvent.click(within(card("New email")).getByRole("button", { name: "Ready" }));
    await waitFor(() => expect(items["01DRAFT"]!.note.fields.status).toBe("approved"));
    await openTab("Failed");
    await screen.findByRole("link", { name: "Lunch" });
    fireEvent.click(within(card("Lunch")).getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(items["01FAILED"]!.note.fields.status).toBe("approved"));
    expect(items["01FAILED"]!.note.fields.error).toBeUndefined();
    await openTab("Rejected");
    await screen.findByRole("link", { name: "Spam" });
    fireEvent.click(within(card("Spam")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deleteNote).toHaveBeenCalledWith("01REJECTED", "v-01REJECTED"));
  });

  it("approves every item in Review after a confirm naming the count", async () => {
    render(() => <Outbox />);
    await screen.findByRole("link", { name: "Re: Roof" });
    fireEvent.click(screen.getByRole("button", { name: "Approve all" }));
    const dialog = screen.getByRole("alertdialog");
    expect(dialog.textContent).toContain("Approve and send 3 items in Review?");
    expect(updateNote).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Approve and send 3 items" }));
    await waitFor(() => expect(updateNote).toHaveBeenCalledTimes(3));
    for (const id of ["01REVIEW", "01REVIEW2", "01EVENTCHG"]) expect(items[id]!.note.fields.status).toBe("approved");
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  });

  it("cancelling Approve all approves nothing", async () => {
    render(() => <Outbox />);
    await screen.findByRole("link", { name: "Re: Roof" });
    fireEvent.click(screen.getByRole("button", { name: "Approve all" }));
    fireEvent.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(updateNote).not.toHaveBeenCalled();
  });

  it("previews an email's headers, body and attachments, and who drafted it", async () => {
    render(() => <Outbox />);
    await screen.findByRole("link", { name: "Re: Roof" });
    const preview = await within(card("Re: Roof")).findByLabelText("Email preview");
    await waitFor(() => expect(preview.textContent).toContain("The roof is fixed."));
    expect(preview.textContent).toContain("ada@example.com");
    expect(preview.textContent).toContain("bo@example.com");
    expect(preview.textContent).toContain("Re: Roof");
    expect(await within(preview).findByRole("link", { name: "roof.jpg" })).toBeTruthy();
    expect(card("Re: Roof").textContent).toContain("Drafted by clerk");
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
    await screen.findByRole("link", { name: "Standup" });
    const change = await within(card("Standup")).findByLabelText("Event preview");
    await waitFor(() => expect(change.querySelector(".outbox-before")?.textContent).toBe("Room 1"));
    expect(change.querySelector(".outbox-after")?.textContent).toBe("Room 9");
    expect(change.textContent).toContain("A change to");
    await openTab("Drafts");
    await screen.findByRole("link", { name: "Retro" });
    const fresh = await within(card("Retro")).findByLabelText("Event preview");
    expect(fresh.textContent).toContain("A new event");
    expect(fresh.textContent).toContain("Room 2");
    expect(fresh.querySelector(".outbox-before")).toBeNull();
  });

  it("shows Waiting for phone while the SMS's command is pending", async () => {
    render(() => <Outbox />);
    await openTab("Sending");
    await screen.findByRole("link", { name: "SMS to Cy" });
    await waitFor(() => expect(card("SMS to Cy").querySelector(".outbox-status")?.textContent).toBe("Waiting for phone"));
    expect(deviceCommands).toHaveBeenCalledWith("dev-pixel");
  });

  it("shows Sending once the phone has the command done", async () => {
    deviceCommands.mockResolvedValue([command("cmd-1", "done")]);
    render(() => <Outbox />);
    await openTab("Sending");
    await screen.findByRole("link", { name: "SMS to Cy" });
    await waitFor(() => expect(deviceCommands).toHaveBeenCalled());
    await Promise.resolve();
    expect(card("SMS to Cy").querySelector(".outbox-status")?.textContent).toBe("Sending");
  });

  it("opens on the item a link names, in the tab of its status", async () => {
    searchParams = { item: "01FAILED" };
    render(() => <Outbox />);
    await waitFor(() => expect(screen.getByRole("tab", { name: "Failed" }).getAttribute("aria-selected")).toBe("true"));
    await screen.findByRole("link", { name: "Lunch" });
    expect(card("Lunch").getAttribute("aria-current")).toBe("true");
  });

  it("opens on the tab ?tab= names, as SMS Reply all opens Drafts", async () => {
    searchParams = { tab: "drafts" };
    render(() => <Outbox />);
    await waitFor(() => expect(screen.getByRole("tab", { name: "Drafts" }).getAttribute("aria-selected")).toBe("true"));
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
