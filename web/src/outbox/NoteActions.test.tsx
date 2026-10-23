import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Note, NoteDoc, TypeDef } from "../api";

// The note page's Outbox buttons (outbox X8): Reply on an email, Propose a change on an event,
// and Send on the user's own draft.

const createNote = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, createNote } };
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

beforeEach(() => {
  hasOutbox = true;
  createNote.mockImplementation((type: string, note: NoteDoc) => Promise.resolve({ id: "01DRAFT", type, version: "v1", wcl: "", note } as Note));
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("the note page's Outbox buttons", () => {
  it("Reply on an email drafts a reply to the sender and opens it", async () => {
    render(() => <NoteOutboxActions id="01EMAIL" type="email" doc={EMAIL} setStatus={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Propose a change" })).toBeNull();
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

  it("Send on a draft item approves it; Approve and send on one in review; Retry on a failed one", () => {
    const setStatus = vi.fn();
    const doc = (status: string): NoteDoc => ({ fields: { title: "x", kind: "email", status }, blocks: [] });
    const r = render(() => <NoteOutboxActions id="01ITEM" type="outbox" doc={doc("draft")} setStatus={setStatus} />);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(setStatus).toHaveBeenCalledWith("approved");
    r.unmount();
    const r2 = render(() => <NoteOutboxActions id="01ITEM" type="outbox" doc={doc("review")} setStatus={setStatus} />);
    fireEvent.click(screen.getByRole("button", { name: "Approve and send" }));
    r2.unmount();
    render(() => <NoteOutboxActions id="01ITEM" type="outbox" doc={doc("failed")} setStatus={setStatus} />);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(setStatus).toHaveBeenLastCalledWith("approved", { error: undefined });
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
  });

  it("offers nothing on a sent item, or without the Outbox type", () => {
    render(() => <NoteOutboxActions id="01ITEM" type="outbox" doc={{ fields: { status: "sent" }, blocks: [] }} setStatus={vi.fn()} />);
    expect(screen.queryAllByRole("button")).toHaveLength(0);
    cleanup();
    hasOutbox = false;
    render(() => <NoteOutboxActions id="01EMAIL" type="email" doc={EMAIL} setStatus={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Reply" })).toBeNull();
  });
});
