import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Note, NoteDoc, TypeDef } from "../api";

// The note page's Outbox buttons (outbox X8, X13): Reply, Reply all and Forward on an email or an
// SMS, Propose a change on an event, and Ready, Approve and send, Reject and Retry on an item.

const createNote = vi.fn();
const note = vi.fn();
const googleStatus = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, createNote, note, googleStatus } };
});
const navigate = vi.fn();
vi.mock("@solidjs/router", () => ({ useNavigate: () => navigate }));
let hasOutbox = true;
vi.mock("../session", async (orig) => {
  const real = await orig<typeof import("../session")>();
  return { ...real, typeDef: (name: string) => (name === "outbox" && hasOutbox ? ({ name } as TypeDef) : undefined) };
});

const { NoteOutboxActions } = await import("./NoteActions");

const EMAIL: NoteDoc = {
  fields: { title: "Lunch on Friday", from_raw: "Ada <ada@example.com>" },
  blocks: [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text: "Noon?" } }] }],
};

/** An email to the user and Bo, copied to Cy, with an attachment. */
const GROUP_EMAIL: NoteDoc = {
  fields: {
    title: "Re: Roof",
    from_raw: "Ada <ada@example.com>",
    to_raw: ["Me <ME@example.com>", "Bo <bo@example.com>"],
    cc_raw: ["cy@example.com", "ADA@example.com"],
    sent: "2026-10-20T09:30:00Z",
    attachments: ["assets/01ASSET/quote.pdf"],
  },
  blocks: [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text: "See the quote." } }] }],
};

/** The notes `api.note` reads: people and a conversation. */
const notes: Record<string, NoteDoc> = {};
const person = (title: string, phones: string[], labels: string[] = []): NoteDoc => ({
  fields: { title, phones, phone_labels: labels },
  blocks: [],
});
const sms = (fields: Record<string, unknown>, text = "On my way"): NoteDoc => ({
  fields: { title: text, channel: "sms", ...fields } as NoteDoc["fields"],
  blocks: [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text } }] }],
});

let made = 0;
beforeEach(() => {
  hasOutbox = true;
  made = 0;
  for (const k of Object.keys(notes)) delete notes[k];
  createNote.mockImplementation((type: string, doc: NoteDoc) => {
    made += 1;
    return Promise.resolve({ id: made === 1 ? "01DRAFT" : `01DRAFT${made}`, type, version: "v1", wcl: "", note: doc } as Note);
  });
  note.mockImplementation((id: string) =>
    notes[id] ? Promise.resolve({ id, type: "x", version: "v1", wcl: "", note: notes[id] } as Note) : Promise.reject(new Error("not found")),
  );
  googleStatus.mockResolvedValue({ account: "me@example.com" });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

/** The draft `createNote` was called with, `n` calls back from the first. */
const drafted = (n = 0): NoteDoc => createNote.mock.calls[n]![1] as NoteDoc;

describe("the note page's Outbox buttons", () => {
  it("Reply on an email drafts a reply to the sender and opens it", async () => {
    render(() => <NoteOutboxActions id="01EMAIL" type="email" doc={EMAIL} setStatus={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Propose a change" })).toBeNull();
    // Sent to no one else: no Reply all.
    expect(screen.queryByRole("button", { name: "Reply all" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Reply" }));
    await waitFor(() => expect(navigate).toHaveBeenCalledWith("/notes/01DRAFT"));
    const [type, doc] = createNote.mock.calls[0]!;
    expect(type).toBe("outbox");
    expect(doc.fields).toMatchObject({
      kind: "email",
      status: "draft",
      drafted_by: "user",
      to: ["ada@example.com"],
      subject: "Re: Lunch on Friday",
      in_reply_to: "01EMAIL",
    });
    expect(doc.blocks[0].blocks.at(-1)).toEqual({ kind: "quote", fields: { text: "Noon?" } });
  });

  it("Reply all on an email writes to the sender, To and Cc, without the Google account or repeats", async () => {
    render(() => <NoteOutboxActions id="01EMAIL" type="email" doc={GROUP_EMAIL} setStatus={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Reply all" }));
    await waitFor(() => expect(navigate).toHaveBeenCalledWith("/notes/01DRAFT"));
    expect(drafted().fields).toMatchObject({
      kind: "email",
      status: "draft",
      drafted_by: "user",
      to: ["ada@example.com", "bo@example.com", "cy@example.com"],
      subject: "Re: Roof",
      in_reply_to: "01EMAIL",
    });
  });

  it("Forward on an email drafts it to no one with Fwd:, the forwarded header, the body and the attachments", async () => {
    render(() => <NoteOutboxActions id="01EMAIL" type="email" doc={GROUP_EMAIL} setStatus={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Forward this email" }));
    await waitFor(() => expect(navigate).toHaveBeenCalledWith("/notes/01DRAFT"));
    const doc = drafted();
    expect(doc.fields).toMatchObject({ kind: "email", status: "draft", to: [], subject: "Fwd: Re: Roof", attachments: ["assets/01ASSET/quote.pdf"] });
    expect(doc.fields.in_reply_to).toBeUndefined();
    const texts = doc.blocks[0]!.blocks!.map((b) => b.fields?.text);
    expect(texts[1]).toBe("---------- Forwarded message ---------");
    expect(texts).toContain("From: Ada <ada@example.com>");
    expect(texts).toContain("Subject: Re: Roof");
    expect(texts).toContain("To: Me <ME@example.com>, Bo <bo@example.com>");
    expect(texts.some((t) => typeof t === "string" && t.startsWith("Date: ") && t.length > 6)).toBe(true);
    expect(doc.blocks[0]!.blocks!.at(-1)).toEqual({ kind: "quote", fields: { text: "See the quote." } });
  });

  it("Reply on an incoming SMS drafts an SMS to the sender's number", async () => {
    notes["01ADA"] = person("Ada", ["+61400000001"]);
    const doc = sms({ direction: "in", from: "01ADA", from_raw: "+61400000001" });
    render(() => <NoteOutboxActions id="01MSG" type="message" doc={doc} setStatus={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Reply" }));
    await waitFor(() => expect(navigate).toHaveBeenCalledWith("/notes/01DRAFT"));
    expect(createNote).toHaveBeenCalledTimes(1);
    expect(drafted().fields).toEqual({ title: "SMS to Ada", kind: "sms", status: "draft", drafted_by: "user", phone: "+61400000001" });
    expect(drafted().fields.device).toBeUndefined();
  });

  it("Reply on an outgoing SMS drafts to its recipient, through the person when the phone gave a name", async () => {
    notes["01BO"] = person("Bo", ["+61400000009", "+61400000002"], ["home", "mobile"]);
    const doc = sms({ direction: "out", to: ["01BO"], to_raw: ["Bo"] });
    render(() => <NoteOutboxActions id="01MSG" type="message" doc={doc} setStatus={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Reply all" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Reply" }));
    await waitFor(() => expect(navigate).toHaveBeenCalledWith("/notes/01DRAFT"));
    expect(drafted().fields).toMatchObject({ kind: "sms", title: "SMS to Bo", phone: "+61400000002" });
  });

  it("Reply all in a group conversation drafts one SMS per other number and opens the Drafts tab", async () => {
    notes["01ADA"] = person("Ada", ["+61400000001"]);
    notes["01BO"] = person("Bo", ["+61400000002"]);
    notes["01CY"] = person("Cy", ["+61 400 000 003"]);
    notes["01CONV"] = { fields: { title: "Group", participants: ["01ADA", "01BO", "01CY"] }, blocks: [] };
    const doc = sms({ direction: "in", from: "01ADA", from_raw: "+61400000001", to_raw: ["+61400000002"], conversation: "01CONV" });
    render(() => <NoteOutboxActions id="01MSG" type="message" doc={doc} setStatus={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Reply all" }));
    await waitFor(() => expect(navigate).toHaveBeenCalledWith("/outbox?tab=drafts"));
    expect(createNote).toHaveBeenCalledTimes(3);
    expect(createNote.mock.calls.map((c) => [(c[1] as NoteDoc).fields.title, (c[1] as NoteDoc).fields.phone])).toEqual([
      ["SMS to Ada", "+61400000001"],
      ["SMS to Bo", "+61400000002"],
      ["SMS to Cy", "+61 400 000 003"],
    ]);
  });

  it("Forward on an SMS drafts one with no number and the text after Fwd:", async () => {
    const doc = sms({ direction: "in", from_raw: "+61400000001" }, "Gate code is 1234");
    render(() => <NoteOutboxActions id="01MSG" type="message" doc={doc} setStatus={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Forward this message" }));
    await waitFor(() => expect(navigate).toHaveBeenCalledWith("/notes/01DRAFT"));
    expect(drafted().fields).toMatchObject({ kind: "sms", status: "draft", drafted_by: "user", phone: "" });
    expect(drafted().blocks[0]!.blocks).toEqual([{ kind: "paragraph", fields: { text: "Fwd: Gate code is 1234" } }]);
  });

  it("Propose a change on an event drafts an event item referencing it", async () => {
    render(() => <NoteOutboxActions id="01EVENT" type="event" doc={{ fields: { title: "Standup" }, blocks: [] }} setStatus={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Reply" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Propose a change" }));
    await waitFor(() => expect(navigate).toHaveBeenCalledWith("/notes/01DRAFT"));
    expect(createNote).toHaveBeenCalledWith("outbox", {
      fields: { title: "Standup", kind: "event", status: "draft", drafted_by: "user", event: "01EVENT" },
      blocks: [],
    });
  });

  it("Ready on a draft item approves it; Approve and send or Reject on one in review; Retry on a failed one", () => {
    const setStatus = vi.fn();
    const doc = (status: string): NoteDoc => ({ fields: { title: "x", kind: "email", status }, blocks: [] });
    const r = render(() => <NoteOutboxActions id="01ITEM" type="outbox" doc={doc("draft")} setStatus={setStatus} />);
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Ready" }));
    expect(setStatus).toHaveBeenCalledWith("approved");
    r.unmount();
    const r2 = render(() => <NoteOutboxActions id="01ITEM" type="outbox" doc={doc("review")} setStatus={setStatus} />);
    expect(screen.queryByRole("button", { name: "Ready" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Approve and send" }));
    expect(setStatus).toHaveBeenLastCalledWith("approved");
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));
    expect(setStatus).toHaveBeenLastCalledWith("rejected");
    r2.unmount();
    render(() => <NoteOutboxActions id="01ITEM" type="outbox" doc={doc("failed")} setStatus={setStatus} />);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(setStatus).toHaveBeenLastCalledWith("approved", { error: undefined });
    expect(screen.queryByRole("button", { name: "Ready" })).toBeNull();
  });

  it("offers nothing on a sent item, or without the Outbox type", () => {
    render(() => <NoteOutboxActions id="01ITEM" type="outbox" doc={{ fields: { status: "sent" }, blocks: [] }} setStatus={vi.fn()} />);
    expect(screen.queryAllByRole("button")).toHaveLength(0);
    cleanup();
    hasOutbox = false;
    render(() => <NoteOutboxActions id="01EMAIL" type="email" doc={EMAIL} setStatus={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Reply" })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Forward this/ })).toBeNull();
  });
});
