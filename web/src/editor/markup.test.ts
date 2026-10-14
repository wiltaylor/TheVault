import { Editor } from "@tiptap/core";
import { afterAll, describe, expect, it } from "vitest";
import type { Block, TemplateItem } from "../api";
import { blocksToDoc, docToBlocks, docToItems, itemsToDoc } from "./doc";
import { editorExtensions } from "./extensions";
import { type InlineNode, fieldRefNames, parseInline, renameFieldRefs, serializeInline } from "./markup";
import { knowTitle } from "./titles";

const ID = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
knowTitle(ID, "The parent");

// Every inline mark of PRD Appendix A, in the canonical form the writer produces.
const MARKS: [string, string][] = [
  ["bold", "a **bold** word"],
  ["italic", "an *italic* word"],
  ["underline", "an ++underlined++ word"],
  ["strike", "a ~~struck~~ word"],
  ["code", "run `vault init` now"],
  ["highlight", "a ==highlighted== word"],
  ["colour", "mark risks {red}in red{/} today"],
  ["every palette colour", "{gray}a{/}{brown}b{/}{orange}c{/}{yellow}d{/}{green}e{/}{blue}f{/}{purple}g{/}{pink}h{/}{red}i{/}"],
  ["link", "see [the docs](https://example.com/docs) first"],
  ["mention", `see [[${ID}]] first`],
  ["labelled mention", `see [[${ID}|the parent]] first`],
  ["line break", "one\ntwo"],
];

// Escapes and mark combinations that a naive writer gets wrong.
const TRICKY: [string, string][] = [
  ["literal asterisks", "2 \\* 3 \\* 4"],
  ["literal backslash", "C:\\\\Users\\\\wil"],
  ["backslash before a letter stays one backslash", "a \\\\n b"],
  ["literal backtick", "a \\` b"],
  ["literal brackets", "\\[not a link\\](x)"],
  ["literal double brackets", "\\[\\[01JB8Z4K2MQ7V3X9T5R6W8Y2ZB\\]\\]"],
  ["literal brace", "\\{red}not red"],
  ["single plus, tilde, equals stay bare", "1+1=2 ~ approx"],
  ["doubled plus is escaped", "c\\+\\+ and a \\=\\= b and \\~\\~"],
  ["plus at the edge of a marked run", "**\\+1** and ++x\\+++"],
  ["bold and italic together", "***both*** words"],
  ["italic then bold", "*a***b**"],
  ["bold then italic", "**a***b*"],
  ["bold then bold italic", "**a*b***"],
  ["italic inside bold", "**bold *both* bold**"],
  ["overlapping marks", "**a ++b** c++"],
  ["code inside bold", "**see `x`**"],
  ["code in a link", "[`cargo test`](https://doc.rust-lang.org)"],
  ["code holds markup characters verbatim", "`a*b [c] {d}`"],
  ["code holds an escaped backtick and backslash", "`a\\`b\\\\c`"],
  ["code with a line break", "`one\ntwo`"],
  ["link with marks in its label", "[**bold** and *it*](https://example.com)"],
  ["link with a closing paren in its URL", "[wiki](https://en.wikipedia.org/wiki/Rust_(language\\))"],
  ["bold across a link boundary", "[**a](https://x.test)b**"],
  ["coloured bold", "{blue}**deep**{/}"],
  ["colour change", "{red}a{/}{blue}b{/}"],
  ["mention inside bold", `**ask [[${ID}]]**`],
  ["mention label with escapes", `[[${ID}|a \\] b \\| c \\\\ d]]`],
  ["mention inside a link label", `[see [[${ID}]]](https://example.com)`],
  ["highlight next to underline", "==a==++b++"],
  ["only marks", "**x**"],
  ["empty", ""],
];

describe("inline markup", () => {
  it.each([...MARKS, ...TRICKY])("%s round-trips through the parser", (_, text) => {
    expect(serializeInline(parseInline(text))).toBe(text);
  });

  it("parses each mark into the editor's marks", () => {
    const marked = (text: string) =>
      parseInline(text)
        .filter((n): n is Extract<InlineNode, { type: "text" }> => n.type === "text" && !!n.marks)
        .map((n) => [n.text, n.marks!.map((m) => ("attrs" in m ? `${m.type}=${Object.values(m.attrs)[0]}` : m.type))]);
    expect(marked("a **b** c")).toEqual([["b", ["bold"]]]);
    expect(marked("*b*")).toEqual([["b", ["italic"]]]);
    expect(marked("++b++")).toEqual([["b", ["underline"]]]);
    expect(marked("~~b~~")).toEqual([["b", ["strike"]]]);
    expect(marked("`b`")).toEqual([["b", ["code"]]]);
    expect(marked("==b==")).toEqual([["b", ["highlight"]]]);
    expect(marked("{green}b{/}")).toEqual([["b", ["colour=green"]]]);
    expect(marked("[b](https://x.test)")).toEqual([["b", ["link=https://x.test"]]]);
    expect(marked("***b***")).toEqual([["b", ["bold", "italic"]]]);
    expect(parseInline(`[[${ID}|lbl]]`)).toEqual([{ type: "mention", attrs: { id: ID, label: "lbl" } }]);
    expect(parseInline(`[[${ID}]]`)).toEqual([{ type: "mention", attrs: { id: ID, label: null } }]);
  });

  it("reads unmatched or unknown markup as text", () => {
    const plain = (text: string) => parseInline(text).map((n) => (n.type === "text" ? n.text : n.type)).join("");
    expect(plain("a ` b")).toBe("a ` b");
    expect(plain("{teal}x")).toBe("{teal}x");
    expect(plain("{/} x")).toBe("{/} x");
    expect(plain("[no link] here")).toBe("[no link] here");
    expect(plain("[[not-a-ulid]]")).toBe("[[not-a-ulid]]");
    expect(plain("a + b")).toBe("a + b");
    expect(plain("a \\q")).toBe("a \\q");
  });

  it("writes markup typed into the editor with the escapes it needs", () => {
    expect(serializeInline([{ type: "text", text: "2*3 [x] `y` {z} \\ c++ a+b" }])).toBe(
      "2\\*3 \\[x\\] \\`y\\` \\{z} \\\\ c\\+\\+ a+b",
    );
    expect(
      serializeInline([
        { type: "text", text: "a", marks: [{ type: "italic" }] },
        { type: "text", text: "b", marks: [{ type: "bold" }, { type: "italic" }] },
        { type: "text", text: "c", marks: [{ type: "bold" }] },
      ]),
    ).toBe("*a**b*c**");
    expect(serializeInline(parseInline("*a**b*c**"))).toBe("*a**b*c**");
  });
});

describe("text to editor to text", () => {
  const editor = new Editor({ element: document.createElement("div"), extensions: editorExtensions() });
  afterAll(() => editor.destroy());

  const roundTrip = (blocks: Block[]): Block[] => {
    editor.commands.setContent(blocksToDoc(blocks));
    return docToBlocks(editor.getJSON());
  };

  it.each([...MARKS, ...TRICKY.filter(([, t]) => t !== "")])("%s survives the editor unchanged", (_, text) => {
    const blocks: Block[] = [{ kind: "paragraph", fields: { text } }];
    expect(roundTrip(blocks)).toEqual(blocks);
  });

  it("keeps headings, lists, quotes and blocks it does not edit", () => {
    const blocks: Block[] = [
      { kind: "heading", fields: { level: 2, text: "Plan **now**" } },
      { kind: "paragraph", fields: { text: `Import from **Capacities** first, then [[${ID}|the parent]].` } },
      {
        kind: "list",
        fields: { style: "todo" },
        blocks: [
          { kind: "item", fields: { text: "Export a sample", checked: true } },
          {
            kind: "item",
            fields: { text: "Map objects to types", checked: false },
            blocks: [{ kind: "item", fields: { text: "People become `person` notes", checked: false } }],
          },
        ],
      },
      {
        kind: "list",
        fields: { style: "numbered" },
        blocks: [
          { kind: "item", fields: { text: "one" } },
          { kind: "item", fields: { text: "two" }, blocks: [{ kind: "item", fields: { text: "two.a" } }] },
        ],
      },
      { kind: "list", fields: { style: "bullet" }, blocks: [{ kind: "item", fields: { text: "*dot*" } }] },
      { kind: "quote", fields: { text: "Make it work,\nmake it ==right==." } },
      { kind: "code", fields: { language: "rust", source: "fn main() {}\n" } },
      { kind: "terminal", fields: { title: "Dry run" }, blocks: [{ kind: "command", fields: { line: "ls" } }] },
      { kind: "heading", fields: { level: 1, text: "End" } },
    ];
    expect(roundTrip(blocks)).toEqual(blocks);
  });

  it("drops the empty paragraph the caret rests in, and an empty document is no blocks", () => {
    expect(roundTrip([])).toEqual([]);
    editor.commands.setContent(blocksToDoc([{ kind: "paragraph", fields: { text: "x" } }]));
    editor.commands.focus("end");
    editor.commands.enter();
    expect(docToBlocks(editor.getJSON())).toEqual([{ kind: "paragraph", fields: { text: "x" } }]);
  });
});

describe("field references", () => {
  it("reads {{name}} as a reference only in template text", () => {
    expect(parseInline("Due {{due}}.", { fieldRefs: true })).toEqual([
      { type: "text", text: "Due " },
      { type: "fieldRef", attrs: { name: "due" } },
      { type: "text", text: "." },
    ]);
    expect(parseInline("Due {{due}}.")).toEqual([{ type: "text", text: "Due {{due}}." }]);
  });

  it("leaves escaped braces, code spans, spaces and non-identifiers as text", () => {
    for (const text of ["\\{{due}}", "`{{due}}`", "{{ due }}", "{{9lives}}", "{{due}", "{{}}"]) {
      expect(fieldRefNames(text), text).toEqual([]);
    }
    expect(fieldRefNames("**{{title}}** and {{red}} and {{a_b2}}")).toEqual(["title", "red", "a_b2"]);
  });

  it("writes a reference back as {{name}}, keeping its marks", () => {
    const nodes = parseInline("**{{title}}** is {{status}}", { fieldRefs: true });
    expect(serializeInline(nodes)).toBe("**{{title}}** is {{status}}");
  });

  it("renames references and turns a removed one into escaped text", () => {
    expect(renameFieldRefs("{{a}} and {{b}}", (n) => (n === "a" ? "x" : null))).toBe("{{x}} and \\{\\{b}}");
    const untouched = "Keeps   *odd* spacing {{a}}";
    expect(renameFieldRefs(untouched, (n) => n)).toBe(untouched);
  });

  it("round-trips template blocks with references and slots through the editor", () => {
    const items: TemplateItem[] = [
      { slot: "title", label: null },
      { block: { kind: "paragraph", fields: { text: "Due {{due}}, see **{{title}}**" } } },
      { slot: "due", label: "When" },
      { block: { kind: "heading", fields: { level: 2, text: "About {{title}}" } } },
    ];
    const doc = itemsToDoc(items);
    expect(doc.content?.[0]).toEqual({ type: "templateSlot", attrs: { slot: "title", label: null } });
    expect(docToItems(doc)).toEqual(items);
  });
});
