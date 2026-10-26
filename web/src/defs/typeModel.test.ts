import { describe, expect, it } from "vitest";
import type { Json } from "../api";
import { type SchemaType, changeKind, fieldDefOf, inverseChoices, isLinkList, isOwnInverse, makeLinkList, newField, problems, takesDefault, typeFromSchema, typeWcl } from "./typeModel";

/** A field as `GET /api/schema` describes it, with the attributes every definition has. */
function def(name: string, kind: string, more: Record<string, Json> = {}): Record<string, Json> {
  return { name, kind, label: name, description: null, required: false, computed: false, hidden: false, multiple: false, default: null, ...more };
}

function typeOf(...fields: Record<string, Json>[]): SchemaType {
  return { name: "project", label: "Project", plural: "Projects", description: null, icon: null, colour: null, fields, templates: [], handlers: [] };
}

/** The `field <name> { ... }` block of the written type file. */
function block(text: string, name: string): string {
  const at = text.indexOf(`field ${name} {\n`);
  expect(at, `no field ${name} in:\n${text}`).toBeGreaterThanOrEqual(0);
  return text.slice(at, text.indexOf("\n}\n", at) + 3);
}

describe("lists and dicts in the type model (types T8)", () => {
  it("reads a list of references two-way and writes it as a list, never with multiple", () => {
    const m = typeFromSchema(
      typeOf(
        def("members", "reference", {
          multiple: true,
          listed: true,
          items: "reference",
          target: "person",
          relation: "two_way",
          inverse: "projects",
          on_delete: "restrict",
        }),
      ),
    );
    const f = m.fields[0]!;
    expect([f.kind, f.item?.kind, f.item?.target, f.item?.relation, f.item?.inverse, f.item?.on_delete]).toEqual([
      "list",
      "reference",
      "person",
      "two_way",
      "projects",
      "restrict",
    ]);
    expect(problems(m)).toEqual([]);
    const text = typeWcl(m);
    expect(block(text, "members")).toBe(`field members {
  kind = "list"
  items = "reference"
  target = "person"
  relation = "two_way"
  inverse = "projects"
  on_delete = "restrict"
}
`);
    expect(text).not.toContain("multiple");
    // The note page edits it as the reference field holding several that it reads as.
    expect(fieldDefOf(f)).toMatchObject({ name: "members", kind: "reference", multiple: true, listed: true, target: "person" });
    // A single reference cannot be a peer group.
    f.item!.relation = "peer";
    expect(problems(m)).toEqual([]);
    changeKind(f, "reference");
    f.relation = "peer";
    f.target = "project";
    expect(problems(m)).toContain("members: a peer reference holds several notes: make it a list of references.");
  });

  it("reads the old multiple = true as a list of that kind", () => {
    const m = typeFromSchema(typeOf(def("files", "asset", { multiple: true, accept: "image" })));
    expect(block(typeWcl(m), "files")).toBe(`field files {
  kind = "list"
  items = "asset"
  accept = "image"
}
`);
  });

  it("reads and writes a list of selects with their option labels and a default", () => {
    const m = typeFromSchema(
      typeOf(
        def("sizes", "select", {
          multiple: true,
          listed: true,
          items: "select",
          options: ["s", "m"],
          option_labels: ["Small", ""],
          default: ["m"],
        }),
      ),
    );
    const f = m.fields[0]!;
    expect(takesDefault(f)).toBe(true);
    expect(block(typeWcl(m), "sizes")).toBe(`field sizes {
  kind = "list"
  default = ["m"]
  items = "select"
  options = ["s", "m"]
  option_labels = ["Small", ""]
}
`);
    f.item!.options = [];
    expect(problems(m)).toContain("sizes: a select field needs options.");
  });

  it("reads and writes a list of date ranges", () => {
    const m = typeFromSchema(
      typeOf(def("spans", "list", { items: "date_range", item: def("spans", "date_range", { of: "datetime" }), labels: null })),
    );
    expect(m.fields[0]!.item).toMatchObject({ kind: "date_range", of: "datetime" });
    expect(takesDefault(m.fields[0]!)).toBe(false);
    expect(block(typeWcl(m), "spans")).toBe(`field spans {
  kind = "list"
  items = "date_range"
  of = "datetime"
}
`);
    expect(fieldDefOf(m.fields[0]!)).toMatchObject({ kind: "list", items: "date_range", item: { kind: "date_range", of: "datetime" } });
  });

  it("writes a list of lists of whole numbers with an item block", () => {
    const m = typeFromSchema(
      typeOf(
        def("grid", "list", {
          items: "list",
          item: def("grid", "list", { items: "int", item: def("grid", "int"), labels: null }),
          labels: null,
        }),
      ),
    );
    expect(m.fields[0]!.item?.item?.kind).toBe("int");
    expect(block(typeWcl(m), "grid")).toBe(`field grid {
  kind = "list"
  item {
    kind = "list"
    items = "int"
  }
}
`);
    // A list of lists of references is one-way only.
    changeKind(m.fields[0]!.item!.item!, "reference");
    const inner = m.fields[0]!.item!.item!;
    inner.target = "person";
    inner.relation = "two_way";
    inner.inverse = "grids";
    expect(problems(m)).toContain("grid item: inside a dictionary or a nested list a reference is one-way.");
    // Of a list of lists, the inner list holding several references is still written as a list.
    expect(block(typeWcl(m), "grid")).toContain(`  item {
    kind = "list"
    items = "reference"
    target = "person"
    relation = "two_way"
    inverse = "grids"
    on_delete = "unlink"
  }`);
  });

  it("reads and writes a dict with keys of a reference, a list and a dict, and other keys of any kind", () => {
    const m = typeFromSchema(
      typeOf(
        def("details", "dict", {
          keys: [
            def("owner", "reference", { label: "Owner", required: true, target: "person", relation: "one_way", inverse: null, on_delete: "unlink" }),
            def("helpers", "reference", { multiple: true, listed: true, items: "reference", target: "person", relation: "one_way", on_delete: "unlink" }),
            def("tags", "list", { items: "string", item: def("tags", "string"), labels: null }),
            def("address", "dict", { keys: [def("street", "string"), def("city", "string", { required: true })], values: null }),
          ],
          values: "any",
        }),
      ),
    );
    const f = m.fields[0]!;
    expect(f.keys.map((k) => [k.name, k.kind, k.item?.kind ?? null])).toEqual([
      ["owner", "reference", null],
      ["helpers", "list", "reference"],
      ["tags", "list", "string"],
      ["address", "dict", null],
    ]);
    expect(f.values).toBe("any");
    expect(problems(m)).toEqual([]);
    expect(block(typeWcl(m), "details")).toBe(`field details {
  kind = "dict"
  values = "any"
  key owner {
    kind = "reference"
    label = "Owner"
    required = true
    target = "person"
    relation = "one_way"
    on_delete = "unlink"
  }
  key helpers {
    kind = "list"
    items = "reference"
    target = "person"
    relation = "one_way"
    on_delete = "unlink"
  }
  key tags {
    kind = "list"
    items = "string"
  }
  key address {
    kind = "dict"
    key street {
      kind = "string"
    }
    key city {
      kind = "string"
      required = true
    }
  }
}
`);
    const d = fieldDefOf(f);
    expect(d.values).toBe("any");
    expect(d.keys?.map((k) => [k.name, k.kind, k.multiple])).toEqual([
      ["owner", "reference", false],
      ["helpers", "reference", true],
      ["tags", "list", false],
      ["address", "dict", false],
    ]);

    // Key names are identifiers, once each; references inside are one-way; a dict needs keys or values.
    f.keys[1]!.item!.relation = "two_way";
    f.keys[1]!.item!.inverse = "helped";
    f.keys[2]!.name = "9tags";
    f.keys[3]!.name = "owner";
    f.keys[3]!.keys = [];
    f.keys[3]!.values = null;
    expect(problems(m)).toEqual([
      "details.helpers item: inside a dictionary or a nested list a reference is one-way.",
      'details: "9tags" is not a key name: use letters, digits and _, not starting with a digit.',
      "details: key owner is used twice.",
      "details.owner: a dictionary needs keys, other keys, or both.",
    ]);
  });

  it("writes a dict's other keys as a kind name when they have no attributes, else as a values block", () => {
    const m = typeFromSchema(
      typeOf(
        def("scores", "dict", { keys: [], values: def("scores", "select", { options: ["low", "high"] }) }),
        def("counts", "dict", { keys: [], values: def("counts", "int") }),
        def("legacy", "dict", { values: "string" }),
      ),
    );
    const text = typeWcl(m);
    expect(block(text, "scores")).toBe(`field scores {
  kind = "dict"
  values {
    kind = "select"
    options = ["low", "high"]
  }
}
`);
    expect(block(text, "counts")).toBe(`field counts {
  kind = "dict"
  values = "int"
}
`);
    expect(block(text, "legacy")).toBe(`field legacy {
  kind = "dict"
  values = "string"
}
`);
    expect(fieldDefOf(m.fields[0]!).values).toMatchObject({ kind: "select", options: ["low", "high"] });
  });

  it("gives a new list an item of text and a new dict entries of text", () => {
    const f = newField("x");
    changeKind(f, "list");
    expect(f.item?.kind).toBe("string");
    const d = newField("y");
    changeKind(d, "dict");
    expect(d.values).toMatchObject({ kind: "string" });
    const m = typeFromSchema(typeOf());
    m.fields.push(f, d);
    expect(typeWcl(m)).toContain(`field x {\n  kind = "list"\n  items = "string"\n}`);
    expect(typeWcl(m)).toContain(`field y {\n  kind = "dict"\n  values = "string"\n}`);
  });
});

describe("a two-way reference that is its own inverse", () => {
  it("passes the model's checks and writes inverse as the field's own name", () => {
    const m = typeFromSchema(
      typeOf(
        def("siblings", "reference", { multiple: true, listed: true, items: "reference", target: "project", relation: "two_way", inverse: "siblings", on_delete: "unlink" }),
        def("partner", "reference", { target: "project", relation: "two_way", inverse: "partner", on_delete: "unlink" }),
      ),
    );
    expect(problems(m)).toEqual([]);
    expect(isOwnInverse("project", "siblings", m.fields[0]!.item!)).toBe(true);
    expect(isOwnInverse("project", "partner", m.fields[1]!)).toBe(true);
    // On another type the same names are an ordinary inverse.
    expect(isOwnInverse("task", "siblings", m.fields[0]!.item!)).toBe(false);
    const text = typeWcl(m);
    expect(block(text, "siblings")).toContain('inverse = "siblings"');
    expect(block(text, "partner")).toContain('inverse = "partner"');
  });

  it("is offered first in the inverse picker when the target is the field's own type", () => {
    const fields = [{ name: "siblings" }, { name: "partner" }];
    expect(inverseChoices("person", "siblings", "person", fields)).toEqual([
      { value: "siblings", label: "siblings (this field: links both ways)" },
      { value: "partner", label: "partner" },
    ]);
    // A field not yet saved is offered too.
    expect(inverseChoices("person", "cousins", "person", fields)[0]).toEqual({ value: "cousins", label: "cousins (this field: links both ways)" });
    // Another type's fields only, for a reference to another type.
    expect(inverseChoices("club", "members", "person", fields)).toEqual([
      { value: "siblings", label: "siblings" },
      { value: "partner", label: "partner" },
    ]);
  });
});

describe("a list of links in the type editor", () => {
  it("is made whole by the preset, written as a list of dicts of a name and a required url, and read back as one", () => {
    const f = newField("links", "string");
    f.default = "x";
    makeLinkList(f);
    expect(isLinkList(f)).toBe(true);
    expect(f.default).toBeUndefined();
    const m = typeFromSchema(typeOf());
    m.fields.push(f);
    expect(problems(m)).toEqual([]);
    expect(block(typeWcl(m), "links")).toBe(
      'field links {\n  kind = "list"\n  item {\n    kind = "dict"\n    key name {\n      kind = "string"\n    }\n    key url {\n      kind = "url"\n      required = true\n    }\n  }\n}\n',
    );
    // As the schema describes it again, keys either way round: still a list of links.
    const item = (keys: Record<string, Json>[]) => def("links", "dict", { keys, values: null });
    const stored = typeFromSchema(
      typeOf(
        def("links", "list", { items: "dict", item: item([def("name", "string"), def("url", "url", { required: true })]) }),
        def("refs", "list", { items: "dict", item: item([def("url", "url"), def("name", "string", { required: true })]) }),
        def("other", "list", { items: "dict", item: item([def("name", "string"), def("url", "string")]) }),
      ),
    );
    expect(stored.fields.map(isLinkList)).toEqual([true, true, false]);
  });
});
