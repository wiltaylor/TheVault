import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { Json } from "./api";
import { templateWcl } from "./defs/typeModel";
import { type WhenField, parseWhen, shownTabs, visibleTabs, whenHolds, whenProblem } from "./when";

interface CaseFile {
  type: string;
  now: string;
  fields: WhenField[];
  notes: Record<string, { id: string; fields: Record<string, Json> }>;
  cases: { note: string; when: string; expected: boolean }[];
}

// The cases crates/query/tests/when.rs runs through the server's query evaluator.
const file: CaseFile = JSON.parse(
  readFileSync(resolve(__dirname, "../../crates/query/tests/when_cases.json"), "utf8"),
);
const clock = { now: Date.parse(file.now) / 1000, utcOffset: 0 };
const noteOf = (name: string) => {
  const n = file.notes[name]!;
  return { id: n.id, type: file.type, fields: n.fields, defs: file.fields };
};

describe("the shared when cases", () => {
  it("covers every operator and function", () => {
    expect(file.cases.length).toBeGreaterThanOrEqual(25);
    const all = file.cases.map((c) => c.when).join(" ");
    for (const s of ["==", "!=", "<", "<=", ">", ">=", "&&", "||", "!", "(", "has(", "under(", "today()", "days(", "now()", "this"]) {
      expect(all).toContain(s);
    }
  });

  for (const c of file.cases) {
    it(`${c.when} on ${c.note} is ${c.expected}`, () => {
      expect(whenHolds(c.when, noteOf(c.note), clock)).toBe(c.expected);
    });
  }
});

describe("parseWhen", () => {
  it("reports where a condition stops parsing", () => {
    expect(() => parseWhen("status == ")).toThrow("1:11: expected an expression, found the end of the query");
    expect(() => parseWhen("done sort title")).toThrow("1:6:");
    expect(() => parseWhen("nope(1)")).toThrow("unknown function `nope`");
  });
});

describe("whenProblem", () => {
  const fields: WhenField[] = [
    { name: "done", kind: "bool" },
    { name: "owner", kind: "reference" },
    { name: "span", kind: "date_range" },
  ];
  it("accepts a condition on the note's own fields", () => {
    expect(whenProblem("!done && span.start < today() && this.done == false && title != \"\"", fields)).toBeNull();
  });
  it("says what is wrong as the server does", () => {
    expect(whenProblem("done ==", fields)).toMatch(/^does not parse: 1:8:/);
    expect(whenProblem("stage == 1", fields)).toBe("`stage` is not a field of the type");
    expect(whenProblem("owner.done", fields)).toBe("`owner.done` reads another note, which a tab condition cannot");
    expect(whenProblem("has(linked_from(t, owner), this)", fields)).toBe(
      "linked_from() reads other notes, which a tab condition cannot",
    );
  });
});

describe("visibleTabs", () => {
  const note = noteOf("full");
  const tabs = [{ title: "Main" }, { title: "Draft", when: 'stage == "draft"' }, { title: "Final", when: 'stage == "final"' }];
  it("leaves out a tab whose condition is false", () => {
    expect(visibleTabs(tabs, note, clock).map((t) => t.tab.title)).toEqual(["Main", "Draft"]);
    expect(visibleTabs(tabs, note, clock).map((t) => t.index)).toEqual([0, 1]);
  });
  it("shows the first tab when every tab is hidden", () => {
    const hidden = tabs.slice(1).map((t) => ({ ...t, when: "done" }));
    expect(visibleTabs(hidden, note, clock)).toEqual([{ tab: hidden[0], index: 0 }]);
  });
});

describe("shownTabs", () => {
  const note = noteOf("full");
  it("gives a card the first tab that shows for the note", () => {
    const tabs = [{ title: "Done", when: "done" }, { title: "Open", when: "!done" }, { title: "Any" }];
    expect(shownTabs(tabs, note, true, clock).map((t) => t.title)).toEqual(["Open"]);
    expect(shownTabs(tabs, note, false, clock).map((t) => t.title)).toEqual(["Open", "Any"]);
  });
});

describe("templateWcl", () => {
  it("writes a tab's when first in the tab, and none for an unconditional tab", () => {
    const wcl = templateWcl({
      name: "default",
      label: null,
      tabs: [
        { title: "Main", items: [{ slot: "title", label: null }] },
        { title: "Draft", when: 'stage == "draft"', items: [] },
        { title: "Blank", when: "  ", items: [] },
      ],
    });
    expect(wcl).toBe(
      'template default {\n  tab "Main" {\n    slot "title"\n  }\n\n  tab "Draft" {\n    when = "stage == \\"draft\\""\n  }\n\n  tab "Blank"\n}\n',
    );
  });

  it("writes a slot's when after its label, so the type editor keeps it", () => {
    const wcl = templateWcl({
      name: "default",
      label: null,
      tabs: [
        {
          title: "Main",
          items: [
            { slot: "due", label: "Due on", when: "!done" },
            { slot: "stage", label: null, when: 'stage != "idea"' },
          ],
        },
      ],
    });
    expect(wcl).toBe(
      'template default {\n  tab "Main" {\n    slot "due" {\n      label = "Due on"\n      when = "!done"\n    }\n    slot "stage" {\n      when = "stage != \\"idea\\""\n    }\n  }\n}\n',
    );
  });
});
