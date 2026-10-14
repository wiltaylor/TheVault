import { describe, expect, it } from "vitest";
import type { TypeDef } from "../api";
import { TITLE_MAX, blocksText, captureDoc, captureTitle, convertDoc, hasContent, localDateTime, textBlocks } from "./model";

const FLEETING: TypeDef = {
  name: "fleeting",
  label: "Fleeting note",
  plural: "Fleeting notes",
  description: null,
  icon: "inbox",
  colour: null,
  builtin: true,
  fields: [
    { name: "body", label: "Body", description: null, kind: "content", required: false, computed: false, multiple: false, compound: true },
    { name: "image", label: "Images", description: null, kind: "asset", required: false, computed: false, multiple: true, compound: false },
    { name: "processed", label: "Processed", description: null, kind: "bool", required: false, computed: false, multiple: false, compound: false, default: false },
  ],
  templates: [],
};

const TASK: TypeDef = {
  ...FLEETING,
  name: "task",
  label: "Task",
  builtin: false,
  fields: [
    { name: "status", label: "Status", description: null, kind: "select", required: false, computed: false, multiple: false, compound: false, options: ["todo"] },
    { name: "notes", label: "Notes", description: null, kind: "content", required: false, computed: false, multiple: false, compound: true },
    { name: "cover", label: "Cover", description: null, kind: "asset", required: false, computed: false, multiple: false, compound: false },
  ],
};

const NOW = new Date(2026, 9, 14, 9, 5);

describe("captureTitle", () => {
  it("is the first line of text, trimmed, with its spaces run together", () => {
    expect(captureTitle("\n\n   Call   the plumber  \nabout the leak", NOW)).toBe("Call the plumber");
  });

  it("cuts a long first line at a word, within the limit", () => {
    const line = "word ".repeat(40).trim();
    const title = captureTitle(line, NOW);
    expect([...title].length).toBeLessThanOrEqual(TITLE_MAX);
    expect(title.endsWith("word…")).toBe(true);
    const unbroken = "x".repeat(200);
    expect(captureTitle(unbroken, NOW)).toBe(`${"x".repeat(TITLE_MAX - 1)}…`);
  });

  it("keeps a line at the limit whole", () => {
    const exact = "y".repeat(TITLE_MAX);
    expect(captureTitle(exact, NOW)).toBe(exact);
  });

  it("names an image-only capture by the local date and time", () => {
    expect(localDateTime(NOW)).toBe("2026-10-14 09:05");
    expect(captureTitle("", NOW)).toBe("Image 2026-10-14 09:05");
    expect(captureTitle("  \n \t ", NOW)).toBe("Image 2026-10-14 09:05");
  });
});

describe("captureDoc", () => {
  it("puts the text in paragraphs, escaping markup, and the images in the image field", () => {
    const doc = captureDoc(FLEETING, "First *line*\nsecond\n\n\nNext [[para]]", ["A1", "A2"], NOW);
    expect(doc.fields).toEqual({ title: "First *line*", processed: false, image: ["A1", "A2"] });
    expect(doc.blocks).toEqual([
      {
        kind: "content",
        labels: ["body"],
        blocks: [
          { kind: "paragraph", fields: { text: "First \\*line\\*\nsecond" } },
          { kind: "paragraph", fields: { text: "Next \\[\\[para\\]\\]" } },
        ],
      },
    ]);
  });

  it("leaves out an empty body and images", () => {
    expect(captureDoc(FLEETING, "Only a title", [], NOW)).toEqual({
      fields: { title: "Only a title", processed: false },
      blocks: [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text: "Only a title" } }] }],
    });
    expect(captureDoc(FLEETING, "", ["A1"], NOW)).toEqual({
      fields: { title: "Image 2026-10-14 09:05", processed: false, image: ["A1"] },
      blocks: [],
    });
  });

  it("knows when there is nothing to save", () => {
    expect(hasContent("", 0)).toBe(false);
    expect(hasContent("  \n", 0)).toBe(false);
    expect(hasContent("x", 0)).toBe(true);
    expect(hasContent("", 1)).toBe(true);
    expect(textBlocks("\n\n")).toEqual([]);
  });
});

describe("convertDoc", () => {
  it("carries the title, body and first image into the target's first content and asset fields", () => {
    const from = captureDoc(FLEETING, "Fix the gate\nIt squeaks", ["A1", "A2"], NOW);
    expect(convertDoc(from, FLEETING, TASK)).toEqual({
      fields: { title: "Fix the gate", cover: "A1" },
      blocks: [{ kind: "content", labels: ["notes"], blocks: from.blocks[0]!.blocks }],
    });
    expect(blocksText(from.blocks[0]!.blocks)).toBe("Fix the gate\nIt squeaks");
  });

  it("drops what the target has no field for", () => {
    const bare: TypeDef = { ...TASK, fields: [] };
    const from = captureDoc(FLEETING, "Fix the gate", ["A1"], NOW);
    expect(convertDoc(from, FLEETING, bare)).toEqual({ fields: { title: "Fix the gate" }, blocks: [] });
  });
});
