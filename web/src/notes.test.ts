import { describe, expect, it } from "vitest";
import type { TypeDef } from "./api";
import { applyBlocks, applyEdits, comparedFields, fieldValue, parseInput, slotField, TITLE_FIELD } from "./notes";

const task: TypeDef = {
  name: "task",
  label: "Task",
  plural: "Tasks",
  description: null,
  icon: null,
  colour: null,
  fields: [
    { ...TITLE_FIELD, name: "estimate", label: "Estimate", kind: "int", required: false },
    { ...TITLE_FIELD, name: "body", label: "Body", kind: "content", required: false, compound: true },
  ],
  templates: [],
};

describe("notes", () => {
  it("resolves the built-in title and the type's fields for slots", () => {
    expect(slotField(task, "title")).toBe(TITLE_FIELD);
    expect(slotField(task, "estimate")?.kind).toBe("int");
    expect(slotField(task, "nope")).toBeUndefined();
  });

  it("finds a compound field's labelled block", () => {
    const doc = {
      fields: { title: "A" },
      blocks: [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph" }] }],
    };
    const body = task.fields[1];
    expect(body && fieldValue(doc, body)).toEqual(doc.blocks[0]);
  });

  it("applies edits and removes cleared fields", () => {
    const doc = { fields: { title: "A", estimate: 3 }, blocks: [] };
    expect(applyEdits(doc, { title: "B", estimate: undefined }).fields).toEqual({ title: "B" });
    expect(doc.fields.estimate).toBe(3);
  });

  it("replaces, adds and removes a content field's block", () => {
    const body = task.fields[1]!;
    const para = (text: string) => ({ kind: "paragraph", fields: { text } });
    const other = { kind: "ink", labels: ["sketch"], blocks: [] };
    const doc = { fields: { title: "A" }, blocks: [{ kind: "content", labels: ["body"], blocks: [para("old")] }, other] };
    expect(applyBlocks(doc, body, [para("new")]).blocks).toEqual([
      { kind: "content", labels: ["body"], blocks: [para("new")] },
      other,
    ]);
    expect(applyBlocks(doc, body, []).blocks).toEqual([other]);
    expect(applyBlocks({ fields: {}, blocks: [] }, body, [para("x")]).blocks).toEqual([
      { kind: "content", labels: ["body"], blocks: [para("x")] },
    ]);
    expect(doc.blocks[0]?.blocks).toEqual([para("old")]);
  });

  it("parses inputs into the field's encoding", () => {
    const estimate = task.fields[0];
    expect(estimate && parseInput(estimate, "4")).toBe(4);
    expect(estimate && parseInput(estimate, "")).toBeUndefined();
    expect(parseInput(TITLE_FIELD, "x")).toBe("x");
  });

  it("compares the title, the type's fields and unknown stored fields, not timestamps", () => {
    const a = { fields: { title: "A", created: "x", legacy: 1 }, blocks: [] };
    const b = { fields: { title: "B", updated: "y" }, blocks: [] };
    expect(comparedFields(task, a, b).map((f) => f.name)).toEqual(["title", "estimate", "body", "legacy"]);
  });
});
