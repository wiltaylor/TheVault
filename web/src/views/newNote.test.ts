import { describe, expect, it } from "vitest";
import type { FieldDef, TypeDef } from "../api";
import { type QueryModel, dateValue, localDay, prefill } from "./newNote";

const field = (name: string, kind: string, more: Partial<FieldDef> = {}): FieldDef => ({
  name,
  label: name,
  description: null,
  kind,
  required: false,
  computed: false,
  multiple: false,
  compound: false,
  ...more,
});

const TASK = {
  name: "task",
  fields: [
    field("title", "string"),
    field("project", "reference", { target: "project" }),
    field("status", "taxonomy", { taxonomy: "status" }),
    field("tags", "taxonomy", { taxonomy: "tags", multiple: true }),
    field("done", "bool"),
    field("flagged", "bool"),
    field("due", "date"),
    field("score", "int", { computed: true }),
  ],
  templates: [],
} as unknown as TypeDef;

const cond = (f: string, op: string, value: unknown = null) => ({ condition: { field: f, op, value } });

describe("prefill", () => {
  it("takes every value the && conditions fix, and nothing a comparison or || leaves open", () => {
    const model = {
      from: ["task"],
      filter: {
        join: "all",
        items: [
          cond("project", "eq", { kind: "this" }),
          cond("status", "under", { kind: "str", value: "Active" }),
          cond("tags", "has", { kind: "str", value: "Home" }),
          cond("done", "is_false"),
          cond("flagged", "is_true"),
          cond("due", "lt", { kind: "today", value: 7 }),
          cond("score", "eq", { kind: "int", value: 3 }),
          cond("project.status", "eq", { kind: "str", value: "Open" }),
          cond("title", "eq", { kind: "str", value: "x" }),
          { group: { join: "any", items: [cond("status", "eq", { kind: "str", value: "Done" })] } },
        ],
      },
    } as QueryModel;
    expect(prefill(model, TASK, "01NOTE")).toEqual({
      project: "01NOTE",
      status: "Active",
      tags: ["Home"],
      done: false,
      flagged: true,
    });
  });

  it("reads into a group joined by &&, and fixes nothing under a top-level ||", () => {
    const nested = {
      from: ["task"],
      filter: { join: "all", items: [{ group: { join: "all", items: [cond("due", "eq", { kind: "today", value: 0 })] } }] },
    } as QueryModel;
    expect(prefill(nested, TASK, undefined)).toEqual({ due: localDay(new Date()) });
    const any = { from: ["task"], filter: { join: "any", items: [cond("done", "is_true")] } } as QueryModel;
    expect(prefill(any, TASK, undefined)).toEqual({});
    expect(prefill({ from: ["task"], filter: null }, TASK, undefined)).toEqual({});
  });
});

describe("dateValue", () => {
  const day = new Date(2026, 9, 21, 15, 30);
  it("gives a date the day, a datetime 9 am for a whole day, and a range over the day or an hour", () => {
    expect(dateValue(field("d", "date"), day, true)).toBe("2026-10-21");
    expect(String(dateValue(field("d", "datetime"), day, true))).toMatch(/^2026-10-21T09:00:00[+-]\d\d:\d\d$/);
    expect(String(dateValue(field("d", "datetime"), day, false))).toMatch(/^2026-10-21T15:30:00[+-]\d\d:\d\d$/);
    expect(dateValue(field("d", "date_range", { of: "date" }), day, true)).toEqual({ start: "2026-10-21", end: "2026-10-21" });
    const r = dateValue(field("d", "date_range", { of: "datetime" }), day, false) as { start: string; end: string };
    expect(r.start).toMatch(/T15:30:00/);
    expect(r.end).toMatch(/T16:30:00/);
  });
});
