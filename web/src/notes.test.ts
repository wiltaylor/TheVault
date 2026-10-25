import { describe, expect, it } from "vitest";
import type { TypeDef } from "./api";
import {
  TITLE_FIELD,
  applyBlocks,
  applyDict,
  applyEdits,
  comparedFields,
  fieldValue,
  missingRequired,
  mergeDocs,
  refValue,
  slotField,
  placedFields,
  propertyFields,
} from "./notes";

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

  it("writes a dict field as its labelled block, and removes the block when emptied", () => {
    const extra = { ...TITLE_FIELD, name: "extra", kind: "dict", compound: true };
    const other = { kind: "content", labels: ["body"], blocks: [] };
    const added = applyDict({ fields: {}, blocks: [other] }, extra, { owner: "Wil" });
    expect(added.blocks).toEqual([other, { kind: "dict", labels: ["extra"], fields: { owner: "Wil" } }]);
    expect(fieldValue(added, extra)).toEqual({ owner: "Wil" });
    expect(applyDict(added, extra, { a: 1 }).blocks[1]).toEqual({ kind: "dict", labels: ["extra"], fields: { a: 1 } });
    expect(applyDict(added, extra, undefined).blocks).toEqual([other]);
  });

  it("compares the title, the type's fields and unknown stored fields, not timestamps", () => {
    const a = { fields: { title: "A", created: "x", legacy: 1 }, blocks: [] };
    const b = { fields: { title: "B", updated: "y" }, blocks: [] };
    expect(comparedFields(task, a, b).map((f) => f.name)).toEqual(["title", "estimate", "body", "legacy"]);
  });
});

describe("the inline note page", () => {
  const typed: TypeDef = {
    ...task,
    fields: [
      ...task.fields,
      { ...TITLE_FIELD, name: "due", label: "Due", kind: "date", required: false },
      { ...TITLE_FIELD, name: "owner", label: "Owner", kind: "string", required: true },
      { ...TITLE_FIELD, name: "size", label: "Size", kind: "int", required: true, default: 1 },
    ],
    templates: [
      {
        name: "default",
        label: null,
        tabs: [
          { title: "A", items: [{ slot: "title", label: null }, { block: { kind: "paragraph", fields: { text: "{{due}}" } } }] },
          { title: "B", items: [{ slot: "body", label: null }] },
        ],
      },
      { name: "card", label: null, tabs: [{ title: "C", items: [{ slot: "estimate", label: null }] }] },
    ],
  };

  it("places the fields the default template has a slot for", () => {
    // A {{due}} reference and a card slot do not count; slots on any default tab do.
    expect([...placedFields(typed)].sort()).toEqual(["body", "title"]);
  });

  it("lists every field in the sidebar but the bodies the page already shows", () => {
    expect(propertyFields(typed).map((f) => f.name)).toEqual(["estimate", "due", "owner", "size"]);
  });

  it("leaves a hidden field out of the sidebar", () => {
    const fields = typed.fields.map((f) => (f.name === "owner" ? { ...f, hidden: true } : f));
    expect(propertyFields({ ...typed, fields }).map((f) => f.name)).toEqual(["estimate", "due", "size"]);
  });

  it("lists placed fields, a dict and the bodies the page does not place, in type order", () => {
    const f = (name: string, kind: string, more: object = {}) => ({ ...TITLE_FIELD, name, label: name, kind, required: false, ...more });
    const slot = (name: string) => ({ slot: name, label: null });
    const t: TypeDef = {
      ...task,
      fields: [
        f("placed", "string"),
        f("free", "int"),
        f("secret", "string", { hidden: true }),
        f("body", "content", { compound: true }),
        f("notes", "content", { compound: true }),
        f("sketch", "ink", { compound: true }),
        f("plan", "canvas", { compound: true }),
        f("extra", "dict", { compound: true }),
      ],
      templates: [{ name: "default", label: null, tabs: [{ title: "Main", items: ["placed", "secret", "body", "sketch", "extra"].map(slot) }] }],
    };
    expect(propertyFields(t).map((x) => x.name)).toEqual(["placed", "free", "notes", "plan", "extra"]);
  });

  it("resolves {{name}} to the title, a field, or null for no such field", () => {
    const doc = { fields: { title: "T", due: "2026-01-02" }, blocks: [] };
    expect(refValue(typed, doc, "title")).toBe("T");
    expect(refValue(typed, doc, "due")).toBe("2026-01-02");
    expect(refValue(typed, doc, "owner")).toBeUndefined();
    expect(refValue(typed, doc, "nope")).toBeNull();
  });

  it("lists the required fields a new note still lacks, skipping ones with defaults", () => {
    expect(missingRequired(typed, { fields: {}, blocks: [] })).toEqual(["Title", "Owner"]);
    expect(missingRequired(typed, { fields: { title: "x", owner: "" }, blocks: [] })).toEqual(["Owner"]);
    expect(missingRequired(typed, { fields: { title: "x", owner: "me" }, blocks: [] })).toEqual([]);
  });
});

describe("mergeDocs", () => {
  const body = (text: string) => ({ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text } }] });
  const base = { fields: { title: "A", status: "todo", updated: "t1" }, blocks: [body("one")] };

  it("keeps each side's changes when they touch different fields, with the server's stamps", () => {
    const mine = { ...base, fields: { ...base.fields, title: "A typed" } };
    const theirs = { fields: { title: "A", status: "done", updated: "t2" }, blocks: [body("two")] };
    expect(mergeDocs(base, mine, theirs)).toEqual({ fields: { title: "A typed", status: "done", updated: "t2" }, blocks: [body("two")] });
  });

  it("takes an unchanged note back with a new stamp as no change", () => {
    const mine = { ...base, fields: { ...base.fields, title: "A typed" } };
    const theirs = { ...base, fields: { ...base.fields, updated: "t2" } };
    expect(mergeDocs(base, mine, theirs)).toEqual({ fields: { title: "A typed", status: "todo", updated: "t2" }, blocks: [body("one")] });
  });

  it("is a clash when both changed the same field differently, unless mine is preferred", () => {
    const mine = { ...base, fields: { ...base.fields, title: "Mine" } };
    const theirs = { ...base, fields: { ...base.fields, title: "Theirs" } };
    expect(mergeDocs(base, mine, theirs)).toBeNull();
    expect(mergeDocs(base, { ...base, blocks: [body("mine")] }, { ...base, blocks: [body("theirs")] })).toBeNull();
    expect(mergeDocs(base, mine, theirs, true)?.fields.title).toBe("Mine");
  });

  it("keeps a field one side removed while the other left it alone removed", () => {
    const mine = { fields: { title: "A", updated: "t1" }, blocks: [] };
    const theirs = { ...base, fields: { ...base.fields, title: "B" } };
    expect(mergeDocs(base, mine, theirs)).toEqual({ fields: { title: "B", updated: "t1" }, blocks: [] });
  });
});
