import { describe, expect, it } from "vitest";
import type { Page, Row } from "../api";
import { FILTERS, SOURCES, type Source, merge, rowTime, sourceQuery, sourcesOf, total } from "./sources";

const source = (key: string): Source => {
  const s = SOURCES.find((x) => x.key === key);
  if (!s) throw new Error(key);
  return s;
};

const row = (id: string, type: string, fields: Row["fields"]): Row => ({ id, type, title: id, fields });
const page = (rows: Row[], n = rows.length): Page => ({ rows, next: null, total: n });

describe("the inbox sources", () => {
  it("list unprocessed captures, mail that is not sent mail, incoming messages and missed calls", () => {
    expect(sourceQuery(source("captures"), false, ["image"])).toBe(
      "from fleeting where !processed sort created desc, id desc select created, processed, image",
    );
    expect(sourceQuery(source("mail"), false)).toBe(
      'from email where !processed && !under(labels, "system/SENT") sort sent desc, id desc',
    );
    expect(sourceQuery(source("messages"), false)).toBe(
      'from message where !processed && direction == "in" sort sent desc, id desc',
    );
    expect(sourceQuery(source("calls"), false)).toBe(
      'from call where !processed && direction == "missed" sort started desc, id desc',
    );
  });

  it("with Show processed keep their incoming rules and drop only !processed", () => {
    expect(sourceQuery(source("captures"), true)).toBe("from fleeting sort created desc, id desc select created, processed");
    expect(sourceQuery(source("mail"), true)).toBe('from email where !under(labels, "system/SENT") sort sent desc, id desc');
    expect(sourceQuery(source("messages"), true)).toBe('from message where direction == "in" sort sent desc, id desc');
    expect(sourceQuery(source("calls"), true)).toBe('from call where direction == "missed" sort started desc, id desc');
  });

  it("have the chips All, Captures, Mail, Messages and Calls, and All lists every source", () => {
    expect(FILTERS.map((f) => f.label)).toEqual(["All", "Captures", "Mail", "Messages", "Calls"]);
    expect(sourcesOf("all").map((s) => s.key)).toEqual(["captures", "mail", "messages", "calls"]);
    expect(sourcesOf("calls").map((s) => s.key)).toEqual(["calls"]);
  });
});

describe("merge", () => {
  const capture = row("01J0000000000000000000000A", "fleeting", { created: "2026-11-05T10:00:00Z" });
  const mail = row("01J0000000000000000000000B", "email", { sent: "2026-11-05T12:00:00Z" });
  const message = row("01J0000000000000000000000C", "message", { sent: "2026-11-05T11:00:00+01:00" });
  const call = row("01J0000000000000000000000D", "call", { started: "2026-11-05T09:30:00Z" });

  const pages = [
    { source: source("captures"), page: page([capture]) },
    { source: source("mail"), page: page([mail], 7) },
    { source: source("messages"), page: page([message], 3) },
    { source: source("calls"), page: page([call]) },
  ];

  it("orders the items of every source by their own time, newest first", () => {
    // The message at 11:00+01:00 is 10:00Z, the same instant as the capture: the larger id goes first.
    expect(merge(pages, 50).map((i) => i.row.id)).toEqual([mail.id, message.id, capture.id, call.id]);
  });

  it("keeps the first `limit` items and counts the whole result", () => {
    expect(merge(pages, 2).map((i) => i.row.type)).toEqual(["email", "message"]);
    expect(total(pages)).toBe(12);
  });

  it("puts an item with no time last", () => {
    const undated = row("01J0000000000000000000000Z", "email", {});
    const merged = merge([{ source: source("mail"), page: page([undated, mail]) }, ...pages.slice(2)], 50);
    expect(merged.at(-1)?.row.id).toBe(undated.id);
    expect(rowTime(source("mail"), undated)).toBeNull();
    expect(rowTime(source("calls"), call)).toBe(Date.parse("2026-11-05T09:30:00Z"));
  });
});
