import { describe, expect, it } from "vitest";
import type { NoteDoc, TypeDef } from "./api";
import { placeholderFormUrl, readPickerQuery, replacePlaceholderInDoc, typeOptions } from "./placeholders";

const ID = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";

function t(name: string, label: string): TypeDef {
  return { name, label, plural: `${label}s`, description: null, icon: null, colour: null, fields: [], templates: [] };
}

const TYPES = [t("task", "Task"), t("project", "Project"), t("person", "Person"), t("meeting", "Meeting"), t("idea", "Idea")];

describe("the types a new placeholder offers", () => {
  it("put the note's own type, then recent ones, then the rest by label, a few at most", () => {
    expect(typeOptions(TYPES, "meeting", ["person", "meeting"], null).map((x) => x.name)).toEqual(["meeting", "person", "idea"]);
    expect(typeOptions(TYPES, undefined, [], null).map((x) => x.name)).toEqual(["idea", "meeting", "person"]);
    expect(typeOptions(TYPES, "gone", ["gone"], null).map((x) => x.name)).toEqual(["idea", "meeting", "person"]);
  });

  it("narrow by `type:`, matches at the start first", () => {
    expect(typeOptions(TYPES, "task", [], "p").map((x) => x.name)).toEqual(["person", "project"]);
    expect(typeOptions(TYPES, "task", [], "e").map((x) => x.name)).toEqual(["idea", "meeting", "person", "project"]);
    expect(typeOptions(TYPES, "task", [], "").length).toBe(5);
  });

  it("read `type:` anywhere in the text, apart from the title", () => {
    expect(readPickerQuery("Fix pump")).toEqual({ title: "Fix pump", typeFilter: null });
    expect(readPickerQuery("Fix pump type:Pro")).toEqual({ title: "Fix pump", typeFilter: "pro" });
    expect(readPickerQuery("type: Fix  pump")).toEqual({ title: "Fix pump", typeFilter: "" });
    expect(readPickerQuery("prototype:x")).toEqual({ title: "prototype:x", typeFilter: null });
  });
});

describe("rewriting a placeholder in a stored note", () => {
  const doc: NoteDoc = {
    fields: { title: "Plans" },
    blocks: [
      {
        kind: "content",
        labels: ["body"],
        blocks: [
          { kind: "code", fields: { source: "[[+task|Fix]]" } },
          { kind: "list", fields: { style: "bullet" }, blocks: [{ kind: "item", fields: { text: "do **[[+task|Fix]]** now" } }] },
          { kind: "paragraph", fields: { text: "[[+task|Fix]] again" } },
        ],
      },
    ],
  };

  it("rewrites the first one in markup text only, leaving the rest", () => {
    const next = replacePlaceholderInDoc(doc, "task", "Fix", ID)!;
    const body = next.blocks[0]!.blocks!;
    expect(body[0]).toBe(doc.blocks[0]!.blocks![0]);
    expect(body[1]!.blocks![0]!.fields!.text).toBe(`do **[[${ID}]]** now`);
    expect(body[2]!.fields!.text).toBe("[[+task|Fix]] again");
    expect(replacePlaceholderInDoc(doc, "task", "Other", ID)).toBeNull();
  });

  it("opens the new-note form with the title and where it came from", () => {
    expect(placeholderFormUrl("task", "Fix & go", ID)).toBe(`/notes/new?type=task&title=Fix+%26+go&from=${ID}`);
    expect(placeholderFormUrl("task", "x", undefined)).toBe("/notes/new?type=task&title=x");
  });
});
