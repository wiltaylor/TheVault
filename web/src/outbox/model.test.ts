import { describe, expect, it } from "vitest";
import type { NoteDoc } from "../api";
import {
  TABS,
  addressOf,
  bodyText,
  changeDraft,
  draftingAgent,
  eventChanges,
  newDraft,
  phoneOf,
  phonesOf,
  replyDraft,
  replySubject,
  smsTargets,
  statusQuery,
  tabByKey,
  tabOf,
} from "./model";

const EMAIL: NoteDoc = {
  fields: { title: "Lunch on Friday", from_raw: "Ada Lovelace <ada@example.com>", sent: "2026-10-20T09:30:00Z" },
  blocks: [
    {
      kind: "content",
      labels: ["body"],
      blocks: [
        { kind: "paragraph", fields: { text: "Shall we meet at **noon**?" } },
        { kind: "paragraph", fields: { text: "Ada" } },
      ],
    },
  ],
};

describe("the Outbox model", () => {
  it("queries each tab's statuses, newest change first", () => {
    expect(statusQuery(["draft"])).toBe('from outbox where status == "draft" sort updated desc, id desc');
    expect(statusQuery(["ready", "sending"])).toBe('from outbox where status == "ready" || status == "sending" sort updated desc, id desc');
  });

  it("has three tabs, Draft, Ready and Error, found by their keys and the older ones", () => {
    expect(TABS.map((t) => t.label)).toEqual(["Draft", "Ready", "Error"]);
    expect(tabOf("sending")).toBe(1);
    expect(tabOf("failed")).toBe(2);
    expect(tabByKey("draft")).toBe(0);
    expect(tabByKey("drafts")).toBe(0);
    expect(tabByKey("error")).toBe(2);
    expect(tabByKey("review")).toBe(-1);
  });

  it("starts each new draft as the user's, with a body for email and SMS", () => {
    const email = newDraft("email");
    expect(email.fields).toEqual({ title: "New email", kind: "email", status: "draft", drafted_by: "user" });
    expect(email.blocks[0]?.labels).toEqual(["body"]);
    expect(newDraft("sms").fields.kind).toBe("sms");
    expect(newDraft("event").blocks).toEqual([]);
  });

  it("replies to the sender with Re: and the body quoted", () => {
    const reply = replyDraft("01EMAIL", EMAIL);
    expect(reply.fields).toMatchObject({
      title: "Re: Lunch on Friday",
      subject: "Re: Lunch on Friday",
      kind: "email",
      status: "draft",
      drafted_by: "user",
      to: ["ada@example.com"],
      in_reply_to: "01EMAIL",
    });
    const body = reply.blocks[0]!.blocks!;
    expect(body[0]).toEqual({ kind: "paragraph", fields: { text: "" } });
    expect(String(body[1]!.fields!.text)).toContain("Ada Lovelace <ada@example.com> wrote:");
    expect(body[2]).toEqual({ kind: "quote", fields: { text: "Shall we meet at **noon**?\nAda" } });
  });

  it("keeps one Re: and reads a bare address", () => {
    expect(replySubject("RE: Lunch")).toBe("RE: Lunch");
    expect(replySubject("Lunch")).toBe("Re: Lunch");
    expect(addressOf("bo@example.com")).toBe("bo@example.com");
    expect(addressOf('"Bo, B" <bo@example.com>')).toBe("bo@example.com");
  });

  it("proposes a change to an event as a draft referencing it", () => {
    const d = changeDraft("01EVENT", { fields: { title: "Standup" }, blocks: [] });
    expect(d.fields).toEqual({ title: "Standup", kind: "event", status: "draft", drafted_by: "user", event: "01EVENT" });
  });

  it("shows an event change as before and after, leaving out empty fields", () => {
    const event: NoteDoc = { fields: { title: "Standup", location: "Room 1", when: { start: "2026-10-26T09:00", end: "2026-10-26T09:15" } }, blocks: [] };
    const item: NoteDoc = { fields: { title: "Standup", kind: "event", location: "Room 2", attendees_raw: ["bo@example.com"] }, blocks: [] };
    expect(eventChanges(item, event)).toEqual([
      { field: "location", label: "Location", before: "Room 1", after: "Room 2" },
      { field: "attendees_raw", label: "Attendees", before: "", after: "bo@example.com" },
    ]);
    const fresh = eventChanges({ fields: { title: "Retro", when: { start: "2026-10-27", end: "2026-10-27" } }, blocks: [] }, null);
    expect(fresh).toEqual([
      { field: "title", label: "Title", before: "", after: "Retro" },
      { field: "when", label: "When", before: "", after: "2026-10-27" },
    ]);
  });

  it("reads the body as plain text and which agent drafted an item", () => {
    expect(bodyText(EMAIL)).toBe("Shall we meet at **noon**?\nAda");
    expect(draftingAgent({ drafted_by: "user" })).toBeNull();
    expect(draftingAgent({})).toBeNull();
    expect(draftingAgent({ drafted_by: "clerk" })).toBe("clerk");
  });
});

describe("a person's phones", () => {
  const person = (title: string, phones: unknown): NoteDoc => ({ fields: { title, phones } as NoteDoc["fields"], blocks: [] });

  it("reads each phone's number and label from the list of records", () => {
    expect(phonesOf(person("Bo", [{ number: "+61400000009", label: "home" }, { number: "+61400000002" }, { label: "no number" }]))).toEqual([
      { number: "+61400000009", label: "home" },
      { number: "+61400000002", label: "" },
    ]);
    expect(phonesOf(person("Cy", undefined))).toEqual([]);
  });

  it("texts the first mobile number, else the first number", () => {
    expect(phoneOf(person("Bo", [{ number: "+61400000009", label: "home" }, { number: "+61400000002", label: "mobile" }]))).toBe("+61400000002");
    expect(phoneOf(person("Bo", [{ number: "+61400000009", label: "home" }, { number: "+61400000002" }]))).toBe("+61400000009");
    expect(phoneOf(person("Cy", []))).toBe("");
    expect(phoneOf(null)).toBe("");
  });

  it("names a number by the person holding it, compared by digits, and adds people with no number given", () => {
    const bo = person("Bo", [{ number: "+61 400 000 002", label: "mobile" }]);
    const cy = person("Cy", [{ number: "+61400000003", label: "work" }]);
    expect(smsTargets(["+61400000002"], [{ doc: bo }, { doc: cy }])).toEqual([
      { number: "+61400000002", name: "Bo" },
      { number: "+61400000003", name: "Cy" },
    ]);
  });
});
