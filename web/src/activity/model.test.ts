// What the Log works out from activity entries (activity Y4): subjects with or without their kind,
// the live filter, turns matched by `detail.turn`, and the day headings.

import { describe, expect, it } from "vitest";
import type { ActivityItem } from "../api";
import { activityOf } from "./live";
import { actorLabel, dayLabel, logHref, matches, mergeItems, nestTurns, scriptRunLine, subjectOf } from "./model";

function item(id: string, over: Partial<ActivityItem> = {}): ActivityItem {
  return {
    id,
    at_ms: 0,
    kind: "ai.tool",
    category: "ai",
    actor_kind: "agent",
    actor: "assistant",
    summary: "",
    subject_kind: "thread",
    subject: "T",
    detail: null,
    ...over,
  };
}

describe("activity entries", () => {
  it("read a subject with or without its kind in front", () => {
    expect(subjectOf({ subject_kind: "note", subject: "note:01J" })).toEqual({ kind: "note", id: "01J" });
    expect(subjectOf({ subject_kind: "note", subject: "01J" })).toEqual({ kind: "note", id: "01J" });
    expect(subjectOf({ subject_kind: null, subject: "outbox:01X" })).toEqual({ kind: "outbox", id: "01X" });
    expect(subjectOf({ subject_kind: null, subject: null })).toBeNull();
  });

  it("pass the filters a live entry must meet", () => {
    const it1 = item("1", { summary: "Searched Groceries", kind: "ai.tool" });
    expect(matches(it1, {})).toBe(true);
    expect(matches(it1, { category: "ai", actor: "agent:assistant", q: "groceries" })).toBe(true);
    expect(matches(it1, { actor: "agent:triage" })).toBe(false);
    expect(matches(it1, { actor: "agent" })).toBe(true);
    expect(matches(it1, { kind: "ai" })).toBe(true);
    expect(matches(it1, { kind: "ai.turn" })).toBe(false);
    expect(matches(it1, { category: "notes" })).toBe(false);
  });

  it("merge newest first, each once", () => {
    expect(mergeItems([item("1"), item("3")], [item("2"), item("3")]).map((i) => i.id)).toEqual(["3", "2", "1"]);
  });

  it("nest by turn number when the entries carry one", () => {
    const rows = nestTurns([
      item("5", { kind: "ai.turn.end", detail: { turn: 2 } }),
      item("4", { kind: "ai.tool", detail: { turn: 2 } }),
      item("3", { kind: "ai.turn.start", detail: { turn: 2 } }),
      item("2", { kind: "ai.tool", detail: { turn: 1 } }),
      item("1", { kind: "ai.turn.start", detail: { turn: 1 } }),
    ]);
    // Turn 1's end is not on the page; its tool call still stays with it.
    expect(rows.map((r) => [r.item.id, r.children.map((c) => c.id)])).toEqual([
      ["3", ["4", "5"]],
      ["1", ["2"]],
    ]);
  });

  it("leave a tool call at the top when its turn's start is not on the page", () => {
    const rows = nestTurns([item("2", { kind: "ai.turn.end" }), item("1")]);
    expect(rows.map((r) => r.item.id)).toEqual(["2", "1"]);
  });

  it("name days and actors", () => {
    const now = new Date(2026, 9, 24, 12).getTime();
    expect(dayLabel(new Date(2026, 9, 24, 1).getTime(), now)).toBe("Today");
    expect(dayLabel(new Date(2026, 9, 23, 23).getTime(), now)).toBe("Yesterday");
    expect(dayLabel(new Date(2026, 9, 20).getTime(), now)).not.toMatch(/Today|Yesterday/);
    expect(actorLabel("session", "owner")).toBe("You");
    expect(actorLabel("agent", "triage", () => "Inbox triage")).toBe("Inbox triage");
    expect(actorLabel("server", "")).toBe("Server");
    expect(logHref("inbox triage")).toBe("/log?category=ai&actor=agent:inbox%20triage");
  });

  it("read an activity event as the entry or {item}", () => {
    expect(activityOf({ item: item("1") })?.id).toBe("1");
    expect(activityOf(item("2"))?.id).toBe("2");
    expect(activityOf(null)).toBeNull();
    expect(activityOf({ item: { id: 3 } })).toBeNull();
  });
});

describe("scriptRunLine", () => {
  it("says what an ai.script entry ran, how it ended and which version (script-tools T6)", () => {
    const vault = item("9", {
      kind: "ai.script",
      detail: { tool: "count_tagged", outcome: "ok", ms: 12, instructions: 340, script: { kind: "vault", script: "count_tagged", hash: "0123456789abcdef", generation: 7 } },
    });
    expect(scriptRunLine(vault)).toBe("count_tagged · ok · 12 ms · 340 instructions · version 0123456789ab (scripts/count_tagged.ws at generation 7)");
    const code = item("10", { kind: "ai.script", detail: { tool: "run_script", outcome: "error", error_kind: "timeout", script: { kind: "code", hash: "ffff" } } });
    expect(scriptRunLine(code)).toBe("run_script · failed (timeout) · version ffff (code)");
    expect(scriptRunLine(item("11", { kind: "ai.tool" }))).toBeNull();
  });
});
