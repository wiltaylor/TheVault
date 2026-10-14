import { describe, expect, it } from "vitest";
import {
  newTerm,
  problems as taxonomyProblems,
  taxonomyFromSchema,
  taxonomyWcl,
  termColour,
  termColours,
  termRenames,
} from "./taxonomyModel";
import {
  type SchemaType,
  emptyType,
  followFields,
  needsDefault,
  newField,
  problems,
  renames,
  typeFromSchema,
  typeWcl,
  unplaced,
} from "./typeModel";
import { wclBlock, wclString, wclValue } from "./wcl";

describe("wcl", () => {
  it("escapes strings with the subset's escapes only", () => {
    expect(wclString('a "b" \\ c\nd\te')).toBe('"a \\"b\\" \\\\ c\\nd\\te"');
  });

  it("writes floats with a decimal point and records with every member", () => {
    expect(wclValue(3, true)).toBe("3.0");
    expect(wclValue(3)).toBe("3");
    expect(wclValue({ lat: -37.5, lng: 144 }, true)).toBe("{ lat: -37.5, lng: 144.0 }");
    expect(wclValue(["a", 1, false])).toBe('["a", 1, false]');
    expect(() => wclValue(null)).toThrow();
  });

  it("writes blocks with bare identifier labels and quoted display labels", () => {
    const qv = {
      kind: "query_view",
      fields: { query: "from task" },
      blocks: [{ kind: "view", labels: ["open"], fields: { kind: "table" } }],
    };
    expect(wclBlock(qv)).toBe('query_view {\n  query = "from task"\n\n  view open {\n    kind = "table"\n  }\n}\n');
    expect(wclBlock({ kind: "tab", labels: ["Main"] })).toBe('tab "Main"\n');
  });
});

const stored: SchemaType = {
  name: "task",
  label: "Task",
  plural: "Tasks",
  description: null,
  icon: null,
  colour: "blue",
  fields: [
    { name: "estimate", kind: "float", label: "estimate", required: true, computed: false, multiple: false, default: 2 },
    {
      name: "parent",
      kind: "reference",
      label: "Parent",
      required: false,
      computed: false,
      multiple: false,
      default: null,
      target: "task",
      relation: "two_way",
      inverse: "children",
      on_delete: "cascade",
    },
    { name: "place", kind: "location", label: "place", required: false, computed: false, multiple: false, map: "geo", image: null },
  ],
  templates: [
    {
      name: "default",
      label: null,
      tabs: [
        { title: "Main", items: [{ slot: "title", label: null }, { slot: "estimate", label: "Hours" }] },
        { title: "Links", items: [{ slot: "parent", label: null }, { block: { kind: "paragraph", fields: { text: "Hi" } } }] },
      ],
    },
  ],
  handlers: [{ event: "before_update", script: "task", function: null, allow_domains: [], secrets: [] }],
};

describe("type model", () => {
  it("writes a stored type back in the documented order", () => {
    expect(typeWcl(typeFromSchema(stored))).toBe(`label = "Task"
plural = "Tasks"
colour = "blue"

field estimate {
  kind = "float"
  required = true
  default = 2.0
}

field parent {
  kind = "reference"
  label = "Parent"
  target = "task"
  relation = "two_way"
  inverse = "children"
  on_delete = "cascade"
}

field place {
  kind = "location"
  map = "geo"
}

template default {
  tab "Main" {
    slot "title"
    slot "estimate" {
      label = "Hours"
    }
  }

  tab "Links" {
    slot "parent"
    paragraph {
      text = "Hi"
    }
  }
}

handler "before_update" {
  script = "task"
}
`);
  });

  it("reads a list of terms back as a list whose items are a taxonomy, and writes it so", () => {
    const tagged: SchemaType = {
      ...stored,
      fields: [
        { name: "tags", kind: "taxonomy", label: "Tags", required: false, computed: false, multiple: true, taxonomy: "tags", items: "taxonomy" },
        { name: "topics", kind: "taxonomy", label: "topics", required: false, computed: false, multiple: true, taxonomy: "tags" },
      ],
      templates: [],
      handlers: [],
    };
    const m = typeFromSchema(tagged);
    expect(m.fields.map((f) => [f.kind, f.items, f.multiple, f.taxonomy])).toEqual([
      ["list", "taxonomy", false, "tags"],
      ["taxonomy", "string", true, "tags"],
    ]);
    const text = typeWcl(m);
    expect(text).toContain(`field tags {
  kind = "list"
  label = "Tags"
  items = "taxonomy"
  taxonomy = "tags"
}`);
    expect(text).toContain(`field topics {
  kind = "taxonomy"
  multiple = true
  taxonomy = "tags"
}`);
    m.fields[0]!.taxonomy = "";
    expect(problems(m)).toContain("tags: choose a taxonomy.");
  });

  it("tracks renames and makes slots follow renamed and removed fields", () => {
    const m = typeFromSchema(stored);
    m.fields[0]!.name = "hours";
    m.fields.splice(1, 1);
    expect(renames(m)).toEqual({ estimate: "hours" });
    const tabs = followFields(m)[0]!.tabs;
    expect(tabs[0]!.items).toEqual([{ slot: "title", label: null }, { slot: "hours", label: "Hours" }]);
    expect(tabs[1]!.items).toEqual([{ block: { kind: "paragraph", fields: { text: "Hi" } } }]);
  });

  it("makes {{name}} references in fixed blocks follow renamed and removed fields", () => {
    const m = typeFromSchema(stored);
    m.templates[0]!.tabs[1]!.items = [
      { block: { kind: "paragraph", fields: { text: "{{title}} takes {{estimate}} hours, {{parent}}" } } },
      { block: { kind: "list", fields: { style: "bullet" }, blocks: [{ kind: "item", fields: { text: "Was {{estimate}}" } }] } },
    ];
    m.fields[0]!.name = "hours";
    m.fields = m.fields.filter((f) => f.name !== "parent");
    const items = followFields(m)[0]!.tabs[1]!.items;
    expect(items[0]).toEqual({ block: { kind: "paragraph", fields: { text: "{{title}} takes {{hours}} hours, \\{\\{parent}}" } } });
    expect(items[1]).toEqual({
      block: { kind: "list", fields: { style: "bullet" }, blocks: [{ kind: "item", fields: { text: "Was {{hours}}" } }] },
    });
  });

  it("makes controls follow renamed fields and drops those of removed fields, in columns too", () => {
    const m = typeFromSchema(stored);
    const ctl = (field: string) => ({ kind: "control", fields: { field, kind: "number" } });
    m.templates[0]!.tabs[1]!.items = [
      { block: ctl("estimate") },
      { block: ctl("parent") },
      { block: { kind: "columns", blocks: [{ kind: "column", blocks: [ctl("estimate"), ctl("parent")] }, { kind: "column" }] } },
    ];
    m.fields[0]!.name = "hours";
    m.fields = m.fields.filter((f) => f.name !== "parent");
    expect(followFields(m)[0]!.tabs[1]!.items).toEqual([
      { block: ctl("hours") },
      { block: { kind: "columns", blocks: [{ kind: "column", blocks: [ctl("hours")] }, { kind: "column", blocks: [] }] } },
    ]);
  });

  it("makes images from fields follow renamed fields, with their alt and caption, and drops those of removed fields", () => {
    const m = typeFromSchema(stored);
    m.templates[0]!.tabs[1]!.items = [
      { block: { kind: "image", fields: { field: "estimate", alt: "{{estimate}}", caption: "Of {{title}}" } } },
      { block: { kind: "image", fields: { field: "parent" } } },
      { block: { kind: "image", fields: { asset: "01JB9A0C3D4E5F6G7H8J9K0M1N", caption: "{{estimate}}" } } },
    ];
    m.fields[0]!.name = "hours";
    m.fields = m.fields.filter((f) => f.name !== "parent");
    expect(followFields(m)[0]!.tabs[1]!.items).toEqual([
      { block: { kind: "image", fields: { field: "hours", alt: "{{hours}}", caption: "Of {{title}}" } } },
      { block: { kind: "image", fields: { asset: "01JB9A0C3D4E5F6G7H8J9K0M1N", caption: "{{hours}}" } } },
    ]);
  });

  it("names new required fields without a default", () => {
    const m = typeFromSchema(stored);
    const owner = newField("owner");
    owner.required = true;
    m.fields.push(owner);
    expect(needsDefault(m)).toEqual(["owner"]);
    owner.default = "me";
    expect(needsDefault(m)).toEqual([]);
    expect(problems(m)).toEqual([]);
  });

  it("reports bad names, duplicates and incomplete kinds", () => {
    const m = emptyType();
    m.label = "X";
    m.fields.push(newField("Bad"), newField("title"), newField("a", "select"), newField("a", "reference"));
    const c = newField("c");
    c.required = true;
    c.computed = true;
    m.fields.push(c);
    expect(problems(m)).toEqual([
      '"Bad" is not a field name: use a-z, 0-9 and _.',
      "title is reserved.",
      "a: a select field needs options.",
      "a is used twice.",
      "a: choose a target type.",
      "c: a computed field cannot be required.",
    ]);
  });

  it("reports handlers without a script and before handlers that ask for HTTP or secrets", () => {
    const m = emptyType();
    m.label = "X";
    m.handlers.push(
      { event: "before_update", script: "", function: null, allow_domains: [], secrets: [] },
      { event: "before_create", script: "gate", function: null, allow_domains: ["example.com"], secrets: [] },
      { event: "after_create", script: "hook", function: "send", allow_domains: ["example.com"], secrets: ["key"] },
    );
    expect(problems(m)).toEqual([
      "Handler 1 (before_update): choose a script.",
      "Handler 2 (before_create): only after_* handlers may reach hosts or read secrets.",
    ]);
    expect(typeWcl(m)).toContain(`handler "after_create" {
  script = "hook"
  function = "send"
  allow_domains = ["example.com"]
  secrets = ["key"]
}`);
  });

  it("lists the fields a template does not place", () => {
    const m = typeFromSchema(stored);
    expect(unplaced(m.templates[0]!, m.fields)).toEqual(["place"]);
  });
});

describe("taxonomy model", () => {
  const status = taxonomyFromSchema({
    name: "status",
    label: "Status",
    description: null,
    terms: [
      { name: "Inbox", path: "Inbox", colour: "gray", description: null, children: [] },
      {
        name: "Active",
        path: "Active",
        colour: null,
        description: null,
        children: [{ name: "Blocked", path: "Active/Blocked", colour: "red", description: null, children: [] }],
      },
    ],
  });

  it("writes the term tree", () => {
    expect(taxonomyWcl(status)).toBe(`label = "Status"

term "Inbox" {
  colour = "gray"
}

term "Active" {
  term "Blocked" {
    colour = "red"
  }
}
`);
  });

  it("gives a term without a colour its nearest coloured ancestor's, and leaves the rest out", () => {
    const tree = taxonomyFromSchema({
      name: "area",
      label: "Area",
      description: null,
      terms: [
        {
          name: "Plant",
          path: "Plant",
          colour: "green",
          description: null,
          children: [
            {
              name: "Electrical",
              path: "Plant/Electrical",
              colour: null,
              description: null,
              children: [
                { name: "Wiring", path: "Plant/Electrical/Wiring", colour: null, description: null, children: [] },
                { name: "Lights", path: "Plant/Electrical/Lights", colour: "yellow", description: null, children: [] },
              ],
            },
          ],
        },
        { name: "Office", path: "Office", colour: null, description: null, children: [] },
      ],
    });
    // The edited tree, with "" for no colour, resolves as the saved one does.
    expect(Object.fromEntries(termColours(tree.terms))).toEqual({
      Plant: "green",
      "Plant/Electrical": "green",
      "Plant/Electrical/Wiring": "green",
      "Plant/Electrical/Lights": "yellow",
    });
    // An edit shows at once: clearing Plant's colour leaves only Lights coloured.
    tree.terms[0]!.colour = "";
    expect([...termColours(tree.terms).keys()]).toEqual(["Plant/Electrical/Lights"]);
    expect(termColour(status.terms, "Active/Blocked")).toBe("red");
    expect(termColour(status.terms, "Active")).toBeUndefined();
  });

  it("writes allow_new for an open taxonomy, which may have no terms", () => {
    const open = taxonomyFromSchema({ name: "tags", label: "Tags", description: null, allow_new: true, terms: [] });
    expect(open.allow_new).toBe(true);
    expect(taxonomyProblems(open)).toEqual([]);
    expect(taxonomyWcl(open)).toBe(`label = "Tags"\nallow_new = true\n`);
    expect(status.allow_new).toBe(false);
    expect(taxonomyProblems({ ...open, allow_new: false })).toEqual(["A taxonomy needs at least one term."]);
  });

  it("records a rename for every stored term whose path changed", () => {
    const m = structuredClone(status);
    m.terms[1]!.name = "Doing";
    const blocked = m.terms[1]!.children.pop()!;
    m.terms.push(blocked);
    m.terms.push(newTerm("Done"));
    expect(termRenames(m)).toEqual({ Active: "Doing", "Active/Blocked": "Blocked" });
    expect(taxonomyProblems(m)).toEqual([]);
    m.terms.push(newTerm("Done"), newTerm("a/b"));
    expect(taxonomyProblems(m)).toEqual(["Done appears twice at the top level.", 'a/b: a term name cannot contain "/".']);
  });
});
