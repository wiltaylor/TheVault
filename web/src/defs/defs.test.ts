import { describe, expect, it } from "vitest";
import { newTerm, problems as taxonomyProblems, taxonomyFromSchema, taxonomyWcl, termRenames } from "./taxonomyModel";
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

  it("tracks renames and makes slots follow renamed and removed fields", () => {
    const m = typeFromSchema(stored);
    m.fields[0]!.name = "hours";
    m.fields.splice(1, 1);
    expect(renames(m)).toEqual({ estimate: "hours" });
    const tabs = followFields(m)[0]!.tabs;
    expect(tabs[0]!.items).toEqual([{ slot: "title", label: null }, { slot: "hours", label: "Hours" }]);
    expect(tabs[1]!.items).toEqual([{ block: { kind: "paragraph", fields: { text: "Hi" } } }]);
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
