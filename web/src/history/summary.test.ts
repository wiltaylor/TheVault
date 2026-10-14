import { describe, expect, it } from "vitest";
import type { NoteDoc } from "../api";
import { changeSummary, relativeTime, sameJson, summaryText } from "./summary";

const p = (text: string) => ({ kind: "paragraph", fields: { text } });

const base: NoteDoc = {
  fields: { title: "Plan", status: "idea", created: "2026-10-01T09:00:00Z", updated: "2026-10-01T09:00:00Z" },
  blocks: [{ kind: "content", labels: ["body"], blocks: [p("one"), p("two")] }],
};

describe("change summary", () => {
  it("says created for the first version", () => {
    expect(changeSummary(null, base)).toEqual({ created: true });
    expect(summaryText(changeSummary(undefined, base))).toBe("created");
  });

  it("names the title and fields that changed, title first, and ignores the server's times", () => {
    const next: NoteDoc = { ...base, fields: { ...base.fields, status: "done", title: "Plan B", updated: "2026-10-02T09:00:00Z" } };
    const s = changeSummary(base, next);
    expect(s).toEqual({ created: false, added: [], removed: [], changed: ["title", "status"] });
    expect(summaryText(s)).toBe("changed: title, status");
  });

  it("names fields added and removed", () => {
    const next: NoteDoc = { ...base, fields: { title: "Plan", due: "2026-11-01" } };
    expect(summaryText(changeSummary(base, next))).toBe("added: due; removed: status");
  });

  it("compares content blocks by position and kind", () => {
    const reordered: NoteDoc = { ...base, blocks: [{ kind: "content", labels: ["body"], blocks: [p("two"), p("one")] }] };
    expect(summaryText(changeSummary(base, reordered))).toBe("changed: body");
    const kind: NoteDoc = { ...base, blocks: [{ kind: "content", labels: ["body"], blocks: [p("one"), { kind: "quote", fields: { text: "two" } }] }] };
    expect(summaryText(changeSummary(base, kind))).toBe("changed: body");
    const more: NoteDoc = {
      ...base,
      blocks: [...base.blocks, { kind: "ink", labels: ["sketch"], fields: { width: 10, height: 10 }, blocks: [] }],
    };
    expect(summaryText(changeSummary(base, more))).toBe("added: sketch");
    expect(summaryText(changeSummary(more, base))).toBe("removed: sketch");
    // Unlabelled blocks pair by their place among blocks of the same kind.
    const a: NoteDoc = { fields: {}, blocks: [p("x"), p("y")] };
    const b: NoteDoc = { fields: {}, blocks: [p("x"), p("z")] };
    expect(summaryText(changeSummary(a, b))).toBe("changed: paragraph 2");
  });

  it("sees title, field and body changes together, and no change when only key order differs", () => {
    const next: NoteDoc = {
      fields: { status: "done", title: "Other" },
      blocks: [{ kind: "content", labels: ["body"], blocks: [p("one")] }],
    };
    expect(summaryText(changeSummary(base, next))).toBe("changed: title, status, body");
    const same: NoteDoc = { fields: { status: "idea", title: "Plan" }, blocks: [{ labels: ["body"], kind: "content", blocks: [p("one"), p("two")] }] };
    expect(summaryText(changeSummary(base, same))).toBe("no changes");
  });

  it("compares JSON whatever its key order", () => {
    expect(sameJson({ a: 1, b: [1, { c: 2 }] }, { b: [1, { c: 2 }], a: 1 })).toBe(true);
    expect(sameJson({ a: 1 }, { a: 1, b: 2 })).toBe(false);
    expect(sameJson([1, 2], [2, 1])).toBe(false);
  });
});

describe("relative time", () => {
  const now = Date.UTC(2026, 9, 14, 12);
  it("reads as how long ago", () => {
    expect(relativeTime(now - 10_000, now)).toBe("just now");
    expect(relativeTime(now - 5 * 60_000, now)).toMatch(/5 minutes ago/);
    expect(relativeTime(now - 3 * 3600_000, now)).toMatch(/3 hours ago/);
    expect(relativeTime(now - 24 * 3600_000, now)).toMatch(/yesterday|1 day ago/);
  });
});
